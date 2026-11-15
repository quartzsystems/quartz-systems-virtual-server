// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use nl::PortVlan;
use vswitch::port::PortSpec;
use vswitch::{Bond, Limits, Switch, Switches, Uplink};

use super::Core;
use super::fake::{FakeEnv, FakeKernel};
use crate::kernel::Kernel as _;
use crate::proto::{Adapter, uplink_state};

const ETH0: &str = "P0000-B00-D02-F0-E0";
const ETH1: &str = "P0000-B00-D03-F0-E0";
const ETH2: &str = "P0000-B00-D04-F0-E0";

fn host() -> Core<FakeKernel, FakeEnv> {
    let kernel = FakeKernel::with(&["eth0", "eth1", "eth2"]);
    let env = FakeEnv::with(&[("eth0", ETH0), ("eth1", ETH1), ("eth2", ETH2)]);
    Core::new(kernel, env, Switches::default())
}

fn ids(r: &[qvsmsg::Issued]) -> Vec<&str> {
    r.iter().map(|e| e.id.as_str()).collect()
}

fn spec(partition: u32, slot: u32, switch: &str, vlan: u16, tagged: &[u16]) -> PortSpec {
    PortSpec {
        partition,
        slot,
        switch: switch.into(),
        vlan,
        tagged: tagged.to_vec(),
        mac: [
            0x02,
            0x51,
            0x56,
            0,
            u8::try_from(partition).unwrap(),
            u8::try_from(slot).unwrap(),
        ],
    }
}

fn uplink(interfaces: &[&str], bond: Option<Bond>, native: Option<u16>) -> Uplink {
    Uplink {
        interfaces: interfaces.iter().map(|s| (*s).to_string()).collect(),
        bond,
        native_vlan: native,
    }
}

#[test]
fn a_switch_is_made_as_a_bridge_in_no_vlan_with_ipv6_off_and_kept() {
    let mut c = host();
    let status = c.create("PRODNET", &[20, 10], 1500).unwrap();
    assert_eq!(status.bridge, "qvbr1");
    assert_eq!(status.uplink_state, uplink_state::NONE);
    let k = &c.kernel;
    let br = k.link("qvbr1").unwrap();
    assert_eq!(br.kind.as_deref(), Some("bridge"));
    assert!(br.up);
    assert!(k.ipv6_off.contains("qvbr1"));
    assert!(k.vlans.get(&br.index).is_none_or(Vec::is_empty));
    let saved = c.env.saved.clone().unwrap();
    assert_eq!(saved.get("PRODNET").unwrap().vlans, [10, 20]);
    assert_eq!(c.env.codes(), ["C2008010"]);
    // Every error at once; nothing made.
    let r = c.create("PRODNET", &[0, 5000], 99).unwrap_err();
    assert_eq!(
        ids(&r),
        [
            qvsmsg::SWITCH_VLAN_INVALID.id,
            qvsmsg::SWITCH_VLAN_INVALID.id,
            qvsmsg::SWITCH_MTU_INVALID.id,
            qvsmsg::SWITCH_EXISTS.id,
        ]
    );
    assert!(c.kernel.link("qvbr2").is_none());
    // The kernel refuses: nothing is kept.
    c.kernel.fail = Some("create_bridge");
    let r = c.create("OTHER", &[1], 1500).unwrap_err();
    assert_eq!(ids(&r), [qvsmsg::SWITCH_NOT_CHANGED.id]);
    assert!(c.switches().get("OTHER").is_none());
}

#[test]
fn the_management_interface_and_another_switchs_are_refused_as_uplinks() {
    let mut c = host();
    c.create("A", &[10, 20], 1500).unwrap();
    c.create("B", &[30], 1500).unwrap();
    let r = c
        .set_uplink("A", &uplink(&[ETH0, "P9"], None, Some(40)))
        .unwrap_err();
    assert_eq!(
        ids(&r),
        [
            qvsmsg::BOND_MODE_NEEDED.id,
            qvsmsg::NATIVE_VLAN_INVALID.id,
            qvsmsg::INTERFACE_MANAGEMENT.id,
            qvsmsg::INTERFACE_MISSING.id,
        ]
    );
    c.set_uplink("A", &uplink(&[ETH1], None, Some(10))).unwrap();
    let r = c.set_uplink("B", &uplink(&[ETH1], None, None)).unwrap_err();
    assert_eq!(ids(&r), [qvsmsg::INTERFACE_TAKEN.id]);
    // An MTU the interface cannot carry.
    c.kernel.max_mtu.insert("eth2".into(), 1500);
    c.change("B", &[30], 9000).unwrap();
    let r = c.set_uplink("B", &uplink(&[ETH2], None, None)).unwrap_err();
    assert_eq!(ids(&r), [qvsmsg::MTU_TOO_LARGE.id]);
}

#[test]
fn an_uplink_is_a_trunk_of_the_switchs_vlans_with_its_native_one_untagged() {
    let mut c = host();
    c.create("A", &[10, 20], 1500).unwrap();
    c.set_uplink("A", &uplink(&[ETH1], None, Some(10))).unwrap();
    let k = &c.kernel;
    let eth1 = k.index("eth1").unwrap();
    assert_eq!(k.master_of("eth1").as_deref(), Some("qvbr1"));
    assert!(k.ipv6_off.contains("eth1"));
    assert_eq!(k.rules.get(&eth1), Some(&nl::PortRules::UPLINK));
    assert_eq!(k.vlans[&eth1], [PortVlan::native(10), PortVlan::tagged(20)]);
    assert_eq!(
        c.status(&c.switches().get("A").cloned().unwrap())
            .uplink_state,
        uplink_state::UP
    );
    // A VLAN added goes on the trunk; the port's own are left alone.
    c.change("A", &[10, 20, 30], 1500).unwrap();
    assert!(c.kernel.vlans[&eth1].contains(&PortVlan::tagged(30)));
    // Moved to a bond of two: the old interface is given back.
    c.set_uplink("A", &uplink(&[ETH1, ETH2], Some(Bond::ActiveBackup), None))
        .unwrap();
    let k = &c.kernel;
    assert_eq!(k.master_of("eth1").as_deref(), Some("qvbo1"));
    assert_eq!(k.master_of("eth2").as_deref(), Some("qvbo1"));
    assert_eq!(k.master_of("qvbo1").as_deref(), Some("qvbr1"));
    let bond = k.index("qvbo1").unwrap();
    assert_eq!(k.bonds[&bond], Bond::ActiveBackup);
    assert!(k.vlans[&bond].iter().all(|v| !v.untagged));
    // Cleared: both back, IPv6 as it was, the bond gone.
    c.clear_uplink("A").unwrap();
    let k = &c.kernel;
    assert!(k.link("qvbo1").is_none());
    assert_eq!(k.master_of("eth1"), None);
    assert!(!k.ipv6_off.contains("eth1"));
    let codes = c.env.codes();
    assert_eq!(codes.iter().filter(|c| **c == "C2008015").count(), 4);
}

#[test]
fn a_port_is_a_locked_tap_in_its_vlans_with_its_address_and_filter() {
    let mut c = host();
    c.create("A", &[10, 20, 30], 1500).unwrap();
    let (given, _file) = c.open_port(&spec(3, 5, "A", 10, &[20])).unwrap();
    assert_eq!(given.tap, "qvt3s5");
    let k = &c.kernel;
    let tap = k.index("qvt3s5").unwrap();
    assert_eq!(k.master_of("qvt3s5").as_deref(), Some("qvbr1"));
    assert_eq!(k.rules[&tap], nl::PortRules::PARTITION);
    assert_eq!(k.vlans[&tap], [PortVlan::native(10), PortVlan::tagged(20)]);
    assert_eq!(k.filters[&tap], [20]);
    assert!(k.ipv6_off.contains("qvt3s5"));
    let statics: Vec<u16> = k
        .fdb
        .iter()
        .filter(|e| e.port == tap)
        .map(|e| e.vlan)
        .collect();
    assert_eq!(statics, [10, 20]);
    assert_eq!(c.env.ports.len(), 1);
    // What a port cannot be.
    let r = c.open_port(&spec(4, 2, "NONE", 10, &[])).unwrap_err();
    assert_eq!(ids(&r), [qvsmsg::ADAPTER_SWITCH_MISSING.id]);
    let mut twin = spec(4, 2, "A", 40, &[]);
    twin.mac = spec(3, 5, "A", 10, &[]).mac;
    let r = c.open_port(&twin).unwrap_err();
    assert_eq!(
        ids(&r),
        [qvsmsg::ADAPTER_VLAN_MISSING.id, qvsmsg::PORT_MAC_IN_USE.id]
    );
    // A step the kernel refuses leaves no tap.
    c.kernel.fail = Some("ingress_filter");
    let r = c.open_port(&spec(4, 2, "A", 10, &[])).unwrap_err();
    assert_eq!(ids(&r), [qvsmsg::PORT_NOT_MADE.id]);
    assert!(c.kernel.link("qvt4s2").is_none());
    c.kernel.fail = None;
    // The limits.
    let mut tight = c.switches().clone();
    tight.limits = Limits {
        ports_per_switch: 1,
        ..Limits::default()
    };
    c.change_limits(tight.limits).unwrap();
    let r = c.open_port(&spec(4, 2, "A", 10, &[])).unwrap_err();
    assert_eq!(ids(&r), [qvsmsg::SWITCH_PORTS_TOO_MANY.id]);
}

#[test]
fn ports_go_with_their_partitions_and_with_their_taps() {
    let mut c = host();
    c.create("A", &[10], 1500).unwrap();
    for (p, s) in [(1, 2), (1, 3), (2, 2), (3, 2)] {
        c.open_port(&spec(p, s, "A", 10, &[])).unwrap();
    }
    assert_eq!(c.close_port(1, 0), 2);
    assert!(c.kernel.link("qvt1s2").is_none());
    // A VMM that ended took its tap: the port is forgotten.
    c.kernel.tap_closed("qvt2s2");
    c.turn(0);
    assert_eq!(c.port_statuses("").len(), 1);
    // The partition manager says which run.
    assert_eq!(c.reconcile(&[]), 1);
    assert!(c.port_statuses("").is_empty());
    assert!(c.env.ports.is_empty());
}

#[test]
fn a_switch_a_profile_names_or_a_port_is_on_stays_unless_forced() {
    let mut c = host();
    c.create("A", &[10, 20], 1500).unwrap();
    c.set_uplink("A", &uplink(&[ETH1], None, None)).unwrap();
    c.env.adapters = Some(vec![Adapter {
        partition: 1,
        name: "PROD".into(),
        profile: "NORMAL".into(),
        slot: 4,
        switch: "A".into(),
        vlan: 20,
        tagged: vec![],
        mac: String::new(),
        required: true,
    }]);
    assert_eq!(
        ids(&c.delete("A", false).unwrap_err()),
        [qvsmsg::SWITCH_NAMED.id]
    );
    assert_eq!(
        ids(&c.rename("A", "B").unwrap_err()),
        [qvsmsg::SWITCH_NAMED.id]
    );
    assert_eq!(
        ids(&c.change("A", &[10], 1500).unwrap_err()),
        [qvsmsg::SWITCH_VLAN_NAMED.id]
    );
    c.open_port(&spec(1, 4, "A", 10, &[])).unwrap();
    assert_eq!(
        ids(&c.delete("A", true).unwrap_err()),
        [qvsmsg::SWITCH_IN_USE.id]
    );
    c.close_port(1, 0);
    // The partition manager does not answer: nothing is risked.
    c.env.adapters = None;
    assert_eq!(
        ids(&c.delete("A", false).unwrap_err()),
        [qvsmsg::SWITCH_NOT_CHANGED.id]
    );
    c.delete("A", true).unwrap();
    let k = &c.kernel;
    assert!(k.link("qvbr1").is_none());
    assert_eq!(k.master_of("eth1"), None);
    assert!(c.switches().switches.is_empty());
    assert!(c.env.codes().contains(&"C2008016"));
}

#[test]
fn an_mtu_raised_goes_to_the_uplink_first_and_lowered_to_the_ports_first() {
    let mut c = host();
    c.create("A", &[10], 1500).unwrap();
    c.set_uplink("A", &uplink(&[ETH1], None, None)).unwrap();
    c.open_port(&spec(1, 2, "A", 10, &[])).unwrap();
    c.kernel.mtus.clear();
    c.change("A", &[10], 9000).unwrap();
    let order: Vec<&str> = c.kernel.mtus.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(order, ["eth1", "qvt1s2", "qvbr1"]);
    c.kernel.mtus.clear();
    c.change("A", &[10], 1500).unwrap();
    let order: Vec<&str> = c.kernel.mtus.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(order, ["qvt1s2", "qvbr1", "eth1"]);
}

#[test]
fn an_uplinks_carrier_lost_and_back_is_said_by_location() {
    let mut c = host();
    c.create("A", &[10], 1500).unwrap();
    c.set_uplink("A", &uplink(&[ETH1, ETH2], Some(Bond::ActiveBackup), None))
        .unwrap();
    c.env.posted.clear();
    c.kernel.carrier("eth1", false);
    c.watch(100);
    assert_eq!(
        c.env.posted,
        [("A2008025".to_string(), format!("A, {ETH1}"))]
    );
    let a = c.switches().get("A").cloned().unwrap();
    assert_eq!(c.status(&a).uplink_state, uplink_state::DEGRADED);
    c.kernel.carrier("eth2", false);
    c.watch(101);
    assert_eq!(c.env.codes(), ["A2008025", "A2008020"]);
    assert_eq!(c.status(&a).uplink_state, uplink_state::DOWN);
    c.kernel.carrier("eth1", true);
    c.watch(102);
    assert_eq!(
        c.env.posted.last().unwrap(),
        &("C2008020".to_string(), format!("A, {ETH1}"))
    );
    c.watch(103);
    assert_eq!(c.env.posted.len(), 3);
}

#[test]
fn a_service_started_again_finds_its_switches_and_ports_and_takes_away_strays() {
    let mut c = host();
    c.create("A", &[10, 20], 1500).unwrap();
    c.set_uplink("A", &uplink(&[ETH1], None, Some(10))).unwrap();
    let opened = spec(1, 2, "A", 10, &[20]);
    c.open_port(&opened).unwrap();
    let state = c.env.saved.clone().unwrap();
    let kept = c.env.ports.clone();
    // The kernel keeps what the old service made, and a stray of its.
    let mut kernel = std::mem::take(&mut c.kernel);
    let stray = kernel.create_bridge("qvbr9", 1500).unwrap();
    let env = FakeEnv::with(&[("eth0", ETH0), ("eth1", ETH1), ("eth2", ETH2)]);
    let mut again = Core::new(kernel, env, state);
    again.start(&kept, true);
    assert!(again.kernel.links.iter().all(|l| l.index != stray));
    assert_eq!(again.kernel.master_of("eth1").as_deref(), Some("qvbr1"));
    let ports = again.port_statuses("A");
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].spec, opened);
    assert!(ports[0].link);
    assert_eq!(again.env.codes(), ["C2008000"]);
    assert!(again.env.posted[0].1.contains("1 partition port(s) found"));
    // The table and who is on which VLAN.
    let fdb = again.fdb("A").unwrap();
    assert!(
        fdb.iter()
            .any(|r| r.partition == 1 && r.slot == 2 && r.vlan == 20 && r.origin == "static")
    );
    let uses = again.vlan_use();
    assert_eq!(uses.len(), 2);
    assert!(uses[0].native && uses[0].ports == [(1, 2, false)]);
    assert_eq!(uses[1].ports, [(1, 2, true)]);
}

#[test]
fn the_interfaces_say_what_each_is() {
    let mut c = host();
    c.create("A", &[10], 1500).unwrap();
    c.create("B", &[10], 1500).unwrap();
    c.set_uplink("A", &uplink(&[ETH1], None, None)).unwrap();
    let what: Vec<String> = c.interfaces().into_iter().map(|i| i.what).collect();
    assert_eq!(what, ["Management", "Uplink of A", "Not used"]);
    c.clear_uplink("A").unwrap();
    c.set_uplink("B", &uplink(&[ETH1, ETH2], Some(Bond::Lacp), None))
        .unwrap();
    let what: Vec<String> = c.interfaces().into_iter().map(|i| i.what).collect();
    assert_eq!(what, ["Management", "Bond member of B", "Bond member of B"]);
}

#[test]
fn a_switch_built_from_state_with_its_interface_gone_is_internal_and_said() {
    let state = Switches {
        limits: Limits::default(),
        switches: vec![Switch {
            name: "A".into(),
            id: 1,
            vlans: vec![10],
            mtu: 1500,
            uplink: Some(uplink(&["P9"], None, None)),
        }],
    };
    let kernel = FakeKernel::with(&["eth0"]);
    let env = FakeEnv::with(&[("eth0", ETH0)]);
    let mut c = Core::new(kernel, env, state);
    c.start(&[], true);
    assert!(c.kernel.link("qvbr1").is_some());
    assert_eq!(c.env.codes(), ["A2008030", "C2008000"]);
    let a = c.switches().get("A").cloned().unwrap();
    assert_eq!(c.status(&a).uplink_state, uplink_state::MISSING);
}

#[test]
fn a_state_file_that_names_the_management_interface_does_not_take_it() {
    let state = Switches {
        limits: Limits::default(),
        switches: vec![Switch {
            name: "A".into(),
            id: 1,
            vlans: vec![10],
            mtu: 1500,
            uplink: Some(uplink(&[ETH0], None, None)),
        }],
    };
    let mut c = Core::new(
        FakeKernel::with(&["eth0", "eth1"]),
        FakeEnv::with(&[("eth0", ETH0), ("eth1", ETH1)]),
        state,
    );
    c.start(&[], true);
    assert_eq!(c.kernel.master_of("eth0"), None);
    assert!(!c.kernel.ipv6_off.contains("eth0"));
    assert!(
        c.env.posted[0].1.contains("management"),
        "{:?}",
        c.env.posted
    );
}
