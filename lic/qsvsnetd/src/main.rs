// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's network service: `base/netd` run as QS/VS. It brings up the
//! management interface and keeps the clock; it listens on nothing.

#[cfg(target_os = "linux")]
fn main() {
    use refcode::post::remote;

    static APPLIANCE: release::Appliance = release::appliance(remote::post, remote::note);
    // It returns only when it could not start.
    let Err(e) = netd::serve::run(&APPLIANCE);
    eprintln!("the network service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsnetd is a service of a QS/VS host and runs on Linux only");
}
