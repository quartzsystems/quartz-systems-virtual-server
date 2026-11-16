// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's network service: `base/netd` run as QS/VS. It brings up the
//! management interface and keeps the clock; it listens on nothing. The
//! interfaces the virtual switches' state names as uplinks are reserved:
//! the management interface is never one of them, even at an IPL before
//! the switch service has taken them. A change of the management
//! interface's line goes to PID 1 and to the QSI host server, which
//! listens on the management addresses.

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

    /// PID 1's fact, and the QSI server's word when it is the management
    /// line: on a thread of its own, so a server that is slow or away
    /// never holds the network service up. A server that is away reads
    /// PID 1's fact when it starts.
    fn note(key: &str, value: &str) {
        remote::note(key, value);
        if key == appliance::NOTE_MANAGEMENT {
            let line = value.to_string();
            let _ = std::thread::Builder::new()
                .name("qsid-word".into())
                .spawn(move || {
                    let path = std::path::Path::new(release::RUN_DIR).join(qsilocal::SOCKET_FILE);
                    let _ = qsilocal::Calls::management(&mut qsilocal::Client::new(path), line);
                });
        }
    }

    static APPLIANCE: release::Appliance = release::appliance(remote::post, note);
    // It returns only when it could not start.
    let Err(e) = netd::serve::run(&APPLIANCE, reserved);
    eprintln!("the network service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsnetd is a service of a QS/VS host and runs on Linux only");
}
