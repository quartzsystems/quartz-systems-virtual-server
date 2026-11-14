// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's switch service: `net/vswitchd` run as QS/VS, with the host's
//! interfaces and their location codes from QS/VS's inventory, the
//! management device from the network service, and the profiles'
//! adapters from the partition manager. PID 1 starts it after the
//! network service and before the partition manager, and stops it after
//! the partition manager: every partition has ended before.

#[cfg(target_os = "linux")]
fn main() {
    use netd::control::Calls as _;
    use partd::proto::Calls as _;
    use vswitchd::serve::{Host, run};
    use vswitchd::{Adapter, Nic};

    fn nics() -> Vec<Nic> {
        hostinv::interfaces(&hostinv::Roots::default())
            .into_iter()
            .map(|i| Nic {
                name: i.name,
                location: i.location,
                mac: i.mac,
                driver: i.driver,
                speed_mbps: i.speed_mbps,
                carrier: i.carrier,
            })
            .collect()
    }

    fn management() -> Option<String> {
        let socket = release::appliance(|_| {}, |_, _| {}).net_socket();
        netd::control::Client::new(socket)
            .status()
            .ok()
            .map(|s| s.device)
            .filter(|d| !d.is_empty())
    }

    fn partitions() -> partd::proto::Client {
        partd::proto::Client::new(partd::proto::socket(release::RUN_DIR))
    }

    fn adapters() -> Result<Vec<Adapter>, String> {
        partitions().adapters(0).map_err(|e| e.to_string())
    }

    fn running() -> Option<Vec<u32>> {
        let listed = partitions().list().ok()?;
        Some(
            listed
                .into_iter()
                .filter(|s| s.state.active())
                .map(|s| s.id)
                .collect(),
        )
    }

    let host = Host {
        post: refcode::post::remote::code,
        run_dir: release::RUN_DIR,
        state_dir: "/data/state",
        nics,
        management,
        adapters,
        running,
    };
    // It returns only when it could not start.
    let e = run(host);
    eprintln!("the switch service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsvswitchd is a service of a QS/VS host and runs on Linux only");
}
