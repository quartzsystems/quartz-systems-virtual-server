// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The orderly shutdown: the services end in reverse order, each with
//! a timeout; then sync, `/data` unmounted, the `D2` code, and power
//! off or restart.

use std::time::{Duration, Instant};

use refcode::post::posting;
use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

use super::{Host, STOP_GRACE};
use crate::ipl::mounts;

/// How long the shutdown waits for the log service to take what PID 1
/// has queued for it.
const LOG_DRAIN: Duration = Duration::from_secs(2);

/// How a shutdown ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The host powers off.
    PowerOff,
    /// The host restarts.
    Restart,
}

impl Host {
    /// Begin the shutdown, asked for by `by`. A second request while
    /// one runs changes nothing.
    pub fn begin_shutdown(&mut self, kind: Kind, by: &str) {
        if self.shutdown.is_some() {
            return;
        }
        self.shutdown = Some(kind);
        let what = match kind {
            Kind::PowerOff => "power off",
            Kind::Restart => "restart",
        };
        self.post(&refcode::SHUTDOWN_STARTED, &format!("{what}, by {by}"));
        // The log service is the last to be stopped, but within a
        // moment: what PID 1 still has for it goes first.
        let give_up = Instant::now() + LOG_DRAIN;
        while !self.log.flush() && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(sup) = &mut self.sup {
            let occurrences = sup.begin_stop(STOP_GRACE, Instant::now());
            self.occurred(occurrences);
        }
    }

    /// The services are gone: leave the disk clean and stop the host.
    pub(super) fn finish_shutdown(&mut self) -> ! {
        let kind = self.shutdown.unwrap_or(Kind::PowerOff);
        sync();
        if self.data_mounted
            && let Err(e) = mounts::unmount_data(release::DATA_DIR)
        {
            self.console
                .line(&format!("{} not unmounted: {e}", release::DATA_DIR));
        }
        sync();
        // The log service is gone and its disk unmounted: this code is
        // on the console only.
        let (code, command) = match kind {
            Kind::PowerOff => (&refcode::POWER_OFF, RebootCommand::PowerOff),
            Kind::Restart => (&refcode::RESTART, RebootCommand::Restart),
        };
        self.console.line(&posting(code, "").line);
        // Let the serial port drain.
        std::thread::sleep(Duration::from_millis(200));
        let failed = reboot(command);
        self.console
            .line(&format!("the host did not stop: {failed:?}"));
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }
}
