// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's storage service: `base/stgd` run as QS/VS, with the disks'
//! serial numbers and location codes from QS/VS's inventory. PID 1
//! starts it before `/data` is mounted and stops it after `/data` is
//! unmounted.

#[cfg(target_os = "linux")]
fn main() {
    use refcode::post::remote;
    use stgd::{DiskIdentity, Host};

    fn inventory() -> Vec<DiskIdentity> {
        hostinv::read(&hostinv::Roots::default(), None)
            .disks
            .into_iter()
            .map(|d| DiskIdentity {
                name: d.name,
                serial: d.serial,
                location: d.location,
            })
            .collect()
    }

    static APPLIANCE: release::Appliance = release::appliance(remote::post, remote::note);
    // It returns only when it could not start.
    let e = stgd::serve::run(&APPLIANCE, Host { inventory });
    eprintln!("the storage service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsstgd is a service of a QS/VS host and runs on Linux only");
}
