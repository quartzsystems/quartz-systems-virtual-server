// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host IPL, in order.

pub mod cmdline;
#[cfg(target_os = "linux")]
mod mounts;

#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use crate::{console::Console, kvm, sys};

/// Seconds between heartbeats on the serial port.
#[cfg(target_os = "linux")]
const HEARTBEAT: Duration = Duration::from_secs(5);

/// Run the IPL and never return: PID 1 must not exit.
#[cfg(target_os = "linux")]
pub fn run() -> ! {
    sys::panic::install();
    sys::signal::block();
    let failed = mounts::mount_all();
    let mut console = Console::open();

    console.line(&refcode::IPL_STARTED.line());
    console.line(&release::banner());
    for (what, why) in &failed {
        console.line(&format!("mount {what} failed: {why}"));
    }
    let line = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    console.line(&cmdline::parse(&line).line());
    console.line(&kvm::probe::probe().to_string());

    let mut next = Instant::now() + HEARTBEAT;
    loop {
        let left = next.saturating_duration_since(Instant::now());
        if sys::signal::wait(left) {
            sys::signal::reap();
        }
        if Instant::now() >= next {
            console.serial(&refcode::HEARTBEAT.line());
            next += HEARTBEAT;
        }
    }
}
