// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 4: time. An SNTP responder run by the harness, three days
//! ahead of the host's clock, moves the guest's.

use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

use appliance::time::now_secs;
use ntp::Clock;
use ntp::clock::system_now;
use release::fact;

use super::{DHCP_GATEWAY, MAC, Run, booted, channel, fact_of, posted, seed_toml};
use crate::qemu::image::seed_volume;
use crate::qemu::{Drive, Launch};

/// How far ahead the responder's clock is, in seconds.
const AHEAD: i64 = 3 * 86_400;
/// How near the guest's clock must come to it, in seconds.
const WITHIN: i64 = 10;

/// The host's clock, ahead; it cannot be adjusted.
struct Ahead;

impl Clock for Ahead {
    fn now(&self) -> i64 {
        system_now() + AHEAD * 1_000_000_000
    }

    fn slew(&self, _: i64) -> Result<(), String> {
        Err("the responder's clock is not adjusted".into())
    }

    fn step(&self, _: i64) -> Result<(), String> {
        Err("the responder's clock is not adjusted".into())
    }
}

/// The suite.
pub fn time(run: &Run<'_>) -> Result<String, String> {
    // QEMU's user network hands what the guest sends to 10.0.2.2 to
    // the host's loopback.
    let socket = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = socket.local_addr().map_err(|e| e.to_string())?.port();
    let clock: Arc<dyn Clock> = Arc::new(Ahead);
    let responder = ntp::server::serve(socket, clock, Arc::new(Mutex::new(None)), false, None)
        .map_err(|e| format!("the SNTP responder: {e}"))?;
    let server = format!("{DHCP_GATEWAY}:{port}");

    let seed = run.file("seed.img");
    let toml = seed_toml(&format!("ntp_servers = [\"{server}\"]\n"), "");
    seed_volume(&seed, release::SEED_LABEL, &toml)?;
    let launch = Launch {
        nic: Some(MAC.to_string()),
        extra: vec![Drive {
            path: seed,
            serial: "QSVSSEED0".into(),
            place: None,
        }],
        ..run.launch()
    };
    let mut guest = booted(run, &launch)?;
    let deadline = run.deadline();
    let before: i64 = fact_of(&channel(&mut guest)?.status()?, fact::TIME)?
        .parse()
        .map_err(|_| "the status's time is not a number")?;
    guest.seen(&posted(&refcode::TIME_SYNCHRONIZED, &server), deadline)?;
    let status = channel(&mut guest)?.status()?;
    let after: i64 = fact_of(&status, fact::TIME)?
        .parse()
        .map_err(|_| "the status's time is not a number")?;
    responder.stop();
    let off = after - (now_secs() + AHEAD);
    if off.abs() > WITHIN {
        return Err(format!(
            "the guest's clock is {off} s from the responder's after synchronizing"
        ));
    }
    Ok(format!(
        "the responder at {server}, {AHEAD} s ahead, moved the guest's clock {} s; it is now {}",
        after - before,
        fact_of(&status, fact::DATE)?
    ))
}
