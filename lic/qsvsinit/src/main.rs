// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS PID 1. Phase 0: mount the kernel's file systems, post the IPL
//! reference code, print the banner, the slot and whether KVM is usable,
//! then heartbeat on the serial port and reap children for ever.

// The parts that need no kernel are built and tested on any host.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod console;
mod ipl;
mod kvm;
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
