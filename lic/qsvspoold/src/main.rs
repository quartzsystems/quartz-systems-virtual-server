// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's pool service: `stg/poold` run as QS/VS, with QS/VS's pool
//! disk names, the disks' serial numbers and location codes from its
//! inventory, and the host's identity, which its pool disks carry.
//! PID 1 starts it after `/data` is mounted and before the partition
//! manager, and stops it after the partition manager.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::Path;

    use partd::proto::Calls as _;
    use poold::serve::{DiskIdentity, Host};

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

    fn owner() -> Option<[u8; 16]> {
        let key = identity::public_key(&Path::new(release::DATA_DIR).join("state")).ok()?;
        let mut id = [0u8; 16];
        id.copy_from_slice(&key[..16]);
        Some(id)
    }

    fn named(pool: &str, name: &str) -> Vec<String> {
        partd::proto::Client::new(partd::proto::socket(release::RUN_DIR))
            .named(pool.to_string(), name.to_string())
            .unwrap_or_default()
    }

    let host = Host {
        names: &release::POOL_DISK,
        load_source: &release::LOAD_SOURCE,
        inventory,
        post: refcode::post::remote::code_about,
        owner,
        run_dir: release::RUN_DIR,
        pools_dir: release::POOLS_DIR,
        state_file: "/data/state/pools.toml",
        named,
    };
    // It returns only when it could not start.
    let e = poold::serve::run(host);
    eprintln!("the pool service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvspoold is a service of a QS/VS host and runs on Linux only");
}
