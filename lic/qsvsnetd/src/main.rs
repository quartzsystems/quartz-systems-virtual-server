// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's network service: `base/netd` run as QS/VS. It brings up the
//! management interface and keeps the clock; it listens on nothing. The
//! interfaces the virtual switches' state names as uplinks are reserved:
//! the management interface is never one of them, even at an IPL before
//! the switch service has taken them.

#[cfg(target_os = "linux")]
fn main() {
    use refcode::post::remote;

    /// The hardware addresses of the interfaces a switch's uplink names.
    fn reserved() -> Vec<[u8; 6]> {
        let Ok(Some(switches)) = state::Store::at("/data/state").load::<vswitch::Switches>() else {
            return Vec::new();
        };
        let uplinks: Vec<String> = switches
            .switches
            .iter()
            .filter_map(|s| s.uplink.as_ref())
            .flat_map(|u| u.interfaces.iter().cloned())
            .collect();
        hostinv::interfaces(&hostinv::Roots::default())
            .into_iter()
            .filter(|i| uplinks.contains(&i.location))
            .filter_map(|i| vswitch::mac::parse(&i.mac))
            .collect()
    }

    static APPLIANCE: release::Appliance = release::appliance(remote::post, remote::note);
    // It returns only when it could not start.
    let Err(e) = netd::serve::run(&APPLIANCE, reserved);
    eprintln!("the network service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsnetd is a service of a QS/VS host and runs on Linux only");
}
