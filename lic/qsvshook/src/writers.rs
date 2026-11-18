// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The `writers` hook: every writer of `/data` that the earlier power
//! pulls did not cover kept busy at once, for the power to be pulled
//! under (`docs/PHASE11.md`, Section 8). Each runs in a thread of its
//! own and asks the service that owns its file, as a console would:
//! the partition manager's definitions, the switch service's switches,
//! the pool service's virtual disks in the pool `WPOOL` (when the
//! harness has made it), the problem service's problems by a code
//! posted, and PID 1's host dumps. The QSI server's event log follows
//! the services' events by itself. A refusal or a service that is not
//! up yet is no matter: the thread waits a little and asks again.

use std::thread;
use std::time::Duration;

use partd::Calls as _;
use poold::proto::Calls as _;
use supervise::control::Calls as _;
use vswitchd::proto::Calls as _;

/// The pool the harness makes for the virtual disks.
pub const POOL: &str = "WPOOL";
/// The switch kept changing.
pub const SWITCH: &str = "WSW";
/// The partitions kept changing.
pub const PARTITIONS: [&str; 4] = ["W1", "W2", "W3", "W4"];
/// The code posted over and over: it opens a problem and counts.
pub const CODE: &str = "A2010030";

/// How long a writer waits between its writes.
const APART: Duration = Duration::from_millis(150);

/// Start every writer.
pub fn start() {
    for writer in [definitions, switches, vdisks, problems, dumps] {
        thread::spawn(move || {
            let mut n = 0u64;
            loop {
                n += 1;
                writer(n);
                thread::sleep(APART);
            }
        });
    }
}

/// A definition changed: each partition's auto start delay.
fn definitions(n: u64) {
    let mut client = crate::partition::relay::client();
    let name = PARTITIONS[usize::try_from(n % 4).unwrap_or(0)];
    let Ok(all) = client.list() else { return };
    let id = match all.iter().find(|p| p.name == name) {
        Some(p) => p.id,
        None => match client.create(name.into()) {
            Ok(Ok(d)) => d.id,
            _ => return,
        },
    };
    if let Ok(Ok(mut definition)) = client.definition(id) {
        definition.auto_start.delay_secs = u32::try_from(n % 600).unwrap_or(0);
        let _ = client.change(definition);
    }
}

/// The switch made, its VLANs changed each time.
fn switches(n: u64) {
    let mut client = vswitchd::proto::Client::new(vswitchd::proto::socket(release::RUN_DIR));
    let vlans = if n.is_multiple_of(2) {
        vec![1]
    } else {
        vec![1, 2]
    };
    match client.change(SWITCH.into(), vlans.clone(), 1500) {
        Ok(Ok(())) => {}
        _ => {
            let _ = client.create(SWITCH.into(), vlans, 1500);
        }
    }
}

/// A virtual disk made and the one before deleted.
fn vdisks(n: u64) {
    let mut client = poold::proto::Client::new(poold::proto::socket(release::RUN_DIR));
    let _ = client.create_vdisk(POOL.into(), format!("W{}", n % 4), 16, false);
    let _ = client.delete_vdisk(POOL.into(), format!("W{}", (n + 2) % 4), false);
}

/// The code posted, a problem opened or counted.
fn problems(n: u64) {
    let mut client = supervise::control::Client::new(release::CONTROL_SOCKET);
    let _ = client.post(CODE.into(), format!("writers {n}"));
}

/// A host dump now and then.
fn dumps(n: u64) {
    if n.is_multiple_of(40) {
        let mut client = supervise::control::Client::new(release::CONTROL_SOCKET);
        let _ = client.take_dump("writers".into());
    }
}
