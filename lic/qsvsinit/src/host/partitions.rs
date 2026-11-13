// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partitions at the host's shutdown: the partition manager is
//! asked to end every one before any service is stopped. The asking
//! and the waiting are another thread's; the loop only looks whether
//! it is done.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use partd::{Calls as _, Client};

/// What each partition is given to end: the partition manager's own
/// grace for a VMM, and a moment more.
const EACH: Duration = Duration::from_secs(partd::manager::END_GRACE_SECS + 3);
/// How often the partitions are looked at.
const PAUSE: Duration = Duration::from_millis(200);

/// How the ending went: how many partitions were ended, or what was
/// still activated when the wait was given up.
pub type Outcome = Result<u32, String>;

/// Ask the partition manager at `socket` to end every partition, and
/// wait until none is activated. The outcome arrives on the channel.
pub fn end_all(socket: PathBuf) -> Receiver<Outcome> {
    let (done, outcome) = mpsc::channel();
    thread::spawn(move || {
        let mut client = Client::new(socket);
        // No partition manager: nothing it started is watched, and
        // there is nobody to ask.
        let Ok(count) = client.end_all() else {
            let _ = done.send(Ok(0));
            return;
        };
        let give_up = Instant::now() + EACH * count.max(1);
        let result = loop {
            let active = client
                .list()
                .map_or(0, |all| all.iter().filter(|p| p.state.active()).count());
            if active == 0 {
                break Ok(count);
            }
            if Instant::now() >= give_up {
                break Err(format!("{active} of {count} still activated"));
            }
            thread::sleep(PAUSE);
        };
        let _ = done.send(result);
    });
    outcome
}
