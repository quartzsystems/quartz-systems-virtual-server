// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's storage service. PID 1 starts it before `/data` is mounted
//! and stops it after `/data` is unmounted. With a mirrored load
//! source it serves the protected set to the kernel through ublk; the
//! device outlives this process, so a restart loses nothing.

#[cfg(target_os = "linux")]
mod serve;

#[cfg(target_os = "linux")]
fn main() {
    let e = serve::run();
    eprintln!("the storage service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsstgd is a service of a QS/VS host and runs on Linux only");
}
