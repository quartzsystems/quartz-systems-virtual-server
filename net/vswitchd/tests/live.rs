// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service's core over the live kernel, with dummy links as the
//! host's interfaces. It needs a network namespace of its own:
//!
//! ```text
//! NL_LIVE=1 unshare -rn cargo test -p vswitchd --test live
//! ```

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

use std::time::Duration;

use refcode::RefCode;
use vswitch::port::PortSpec;
use vswitch::{Bond, Switches, Uplink};
use vswitchd::kernel::linux::Linux;
use vswitchd::manager::{Core, Env};
use vswitchd::{Adapter, Nic};

struct Host {
    posted: Vec<String>,
}

impl Env for Host {
    fn now(&self) -> u64 {
        0
    }
    fn post(&mut self, code: &'static RefCode, detail: &str) {
        self.posted.push(format!("{} {detail}", code.code));
    }
    fn log(&mut self, _: &str) {}
    fn nics(&mut self) -> Vec<Nic> {
        ["d0", "d1", "d2"]
            .iter()
            .enumerate()
            .map(|(i, n)| Nic {
                name: (*n).to_string(),
                location: format!("L{i}"),
                ..Nic::default()
            })
            .collect()
    }
    fn management(&mut self) -> Option<String> {
        Some("d0".into())
    }
    fn adapters(&mut self) -> Result<Vec<Adapter>, String> {
        Ok(Vec::new())
    }
    fn save(&mut self, _: &Switches) -> Result<(), String> {
        Ok(())
    }
    fn save_ports(&mut self, _: &[PortSpec]) {}
}

#[test]
fn a_switch_with_a_bonded_uplink_and_a_port_is_built_in_the_kernel() {
    if std::env::var_os("NL_LIVE").is_none() {
        return;
    }
    let mut nl = nl::Nl::open().unwrap();
    for n in ["d0", "d1", "d2"] {
        let l = nl.rtnl().create_simple(n, "dummy").unwrap();
        nl.rtnl().set_up(l.index, true).unwrap();
    }
    let mut core = Core::new(
        Linux::open().unwrap(),
        Host { posted: Vec::new() },
        Switches::default(),
    );
    core.start(&[], true);
    core.create("PRODNET", &[10, 20], 1500).unwrap();
    let r = core
        .set_uplink(
            "PRODNET",
            &Uplink {
                interfaces: vec!["L0".into()],
                bond: None,
                native_vlan: None,
            },
        )
        .unwrap_err();
    assert_eq!(r[0].id, qvsmsg::INTERFACE_MANAGEMENT.id);
    core.set_uplink(
        "PRODNET",
        &Uplink {
            interfaces: vec!["L1".into(), "L2".into()],
            bond: Some(Bond::ActiveBackup),
            native_vlan: Some(10),
        },
    )
    .unwrap();
    let bridge = nl.rtnl().link_by_name("qvbr1").unwrap();
    let bond = nl.rtnl().link_by_name("qvbo1").unwrap();
    assert_eq!(bond.master, Some(bridge.index));
    for d in ["d1", "d2"] {
        assert_eq!(nl.rtnl().link_by_name(d).unwrap().master, Some(bond.index));
        assert!(nl.ipv6_disabled(d));
    }
    let vlans = nl.vlans().unwrap();
    let on_bond = vlans.iter().find(|v| v.index == bond.index).unwrap();
    assert!(on_bond.vlans.contains(&nl::PortVlan::native(10)));
    assert!(on_bond.vlans.contains(&nl::PortVlan::tagged(20)));
    let own = vlans.iter().find(|v| v.index == bridge.index);
    assert!(own.is_none_or(|v| v.vlans.is_empty()), "{own:?}");

    let spec = PortSpec {
        partition: 1,
        slot: 2,
        switch: "PRODNET".into(),
        vlan: 20,
        tagged: vec![10],
        mac: [0x02, 0x51, 0x56, 0, 1, 2],
    };
    let (given, file) = core.open_port(&spec).unwrap();
    let tap = nl.rtnl().link_by_name(&given.tap).unwrap();
    assert_eq!(tap.master, Some(bridge.index));
    let fdb = nl.fdb(bridge.index).unwrap();
    assert!(
        fdb.iter()
            .any(|e| e.port == tap.index && e.vlan == 20 && e.origin == nl::Origin::Static)
    );
    assert_eq!(core.port_statuses("").len(), 1);
    // An MTU raised while the port is open; the dummies carry it.
    core.change("PRODNET", &[10, 20, 30], 9000).unwrap();
    assert_eq!(nl.rtnl().link_by_name(&given.tap).unwrap().mtu, 9000);
    assert_eq!(nl.rtnl().link_by_name("d1").unwrap().mtu, 9000);
    // The VMM's file closed: the tap and its port go.
    drop(file);
    std::thread::sleep(Duration::from_millis(100));
    core.turn(0);
    assert!(core.port_statuses("").is_empty());
    assert!(nl.rtnl().link_by_name("qvt1s2").is_err());
    // Back to internal, then gone: the interfaces are given back.
    core.clear_uplink("PRODNET").unwrap();
    assert!(nl.rtnl().link_by_name("qvbo1").is_err());
    assert_eq!(nl.rtnl().link_by_name("d1").unwrap().master, None);
    assert!(!nl.ipv6_disabled("d1"));
    core.delete("PRODNET", false).unwrap();
    assert!(nl.rtnl().link_by_name("qvbr1").is_err());
}
