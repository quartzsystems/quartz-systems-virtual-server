// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An IPL from installation media: the installer runs in place of the
//! service table and has the display and the serial port to itself.
//! When it ends the host restarts or powers off, as the installer says
//! by its exit status; an installer that ends any other way is started
//! again.

use std::fs::OpenOptions;
use std::process::{Command, Stdio};
use std::time::Duration;

use rustix::fs::sync;
use rustix::system::{RebootCommand, reboot};

use crate::host::Host;

/// The installer.
const INSTALLER: &str = "/qsvs/bin/qsvsinstall";
/// Where what the installer says besides its screens is kept.
const LOG: &str = "/run/install.log";
/// The installer's exit status for "restart the host".
const RESTART: i32 = 0;
/// The installer's exit status for "power the host off".
const POWER_OFF: i32 = 10;

fn log() -> Stdio {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

/// Run the installer until it says how the host is to stop.
pub fn run(mut host: Host) -> ! {
    loop {
        host.line("The installer has the display and the serial port from here");
        let mut command = Command::new(INSTALLER);
        command
            .env_clear()
            .stdin(Stdio::null())
            .stdout(log())
            .stderr(log());
        // PID 1 blocks the signals it reads from a descriptor.
        let ended = supervise::signal::unblocked(&mut command)
            .spawn()
            .and_then(|mut child| child.wait());
        let how = match ended {
            Ok(status) => match status.code() {
                Some(RESTART) => stop(&mut host, RebootCommand::Restart, "restarting"),
                Some(POWER_OFF) => stop(&mut host, RebootCommand::PowerOff, "powering off"),
                _ => status.to_string(),
            },
            Err(e) => e.to_string(),
        };
        host.line(&format!(
            "The installer ended unexpectedly ({how}); it starts again"
        ));
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn stop(host: &mut Host, command: RebootCommand, what: &str) -> ! {
    host.line(&format!("The installer has ended: {what}"));
    sync();
    // Let the serial port drain.
    std::thread::sleep(Duration::from_millis(200));
    let failed = reboot(command);
    host.line(&format!("the host did not stop: {failed:?}"));
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
