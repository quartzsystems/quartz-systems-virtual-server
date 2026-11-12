// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS PID 1. The IPL in order, each step posting its reference
//! code; then the base system: the service table supervised, the
//! control socket, the status screen, slot confirmation, and an orderly
//! shutdown on request, on `SIGTERM` and on the power button.

// The parts that need no kernel are built and tested on any host.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod console;
#[cfg(all(target_os = "linux", test_hooks))]
mod hooks;
#[cfg(target_os = "linux")]
mod host;
mod ipl;
mod kvm;
mod slot;
#[cfg(target_os = "linux")]
mod sys;

#[cfg(target_os = "linux")]
fn main() {
    ipl::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsinit is PID 1 of a QS/VS host and runs on Linux only");
}
