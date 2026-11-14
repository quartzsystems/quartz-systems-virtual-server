// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Every error of the switches, each a message. What needs the host's
//! interfaces (that one is there, that it is not the management
//! interface, that it carries the MTU) is the service's.

use qvsmsg::Issued;

use crate::switch::{
    LIMIT_RANGES, Limits, MAX_MTU, MAX_UPLINK_INTERFACES, MIN_MTU, Switch, Switches, Uplink,
};

/// Whether `name` is a switch's name: 1 to 10 of `A` to `Z`, `0` to
/// `9` and `_`, a letter first, as a partition's.
pub fn valid_name(name: &str) -> bool {
    let b = name.as_bytes();
    (1..=10).contains(&b.len())
        && b[0].is_ascii_uppercase()
        && b.iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
}

/// Whether `vlan` is a VLAN ID.
pub fn valid_vlan(vlan: u16) -> bool {
    (1..=4094).contains(&vlan)
}

/// The errors of one switch's own fields, under `limits`.
pub fn switch(s: &Switch, limits: &Limits) -> Vec<Issued> {
    let mut errors = Vec::new();
    if !valid_name(&s.name) {
        errors.push(qvsmsg::SWITCH_NAME_INVALID.with(&[&s.name]));
    }
    let mut seen = Vec::new();
    for v in &s.vlans {
        if !valid_vlan(*v) || seen.contains(v) {
            errors.push(qvsmsg::SWITCH_VLAN_INVALID.with(&[&v.to_string(), &s.name]));
        }
        seen.push(*v);
    }
    if s.vlans.len() > limits.vlans_per_switch as usize {
        errors.push(qvsmsg::SWITCH_VLANS_TOO_MANY.with(&[
            &s.name,
            &s.vlans.len().to_string(),
            &limits.vlans_per_switch.to_string(),
        ]));
    }
    if !(MIN_MTU..=MAX_MTU).contains(&s.mtu) {
        errors.push(qvsmsg::SWITCH_MTU_INVALID.with(&[&s.mtu.to_string(), &s.name]));
    }
    if let Some(u) = &s.uplink {
        errors.extend(uplink(s, u));
    }
    errors
}

/// The errors of an uplink of `s` that need no look at the host.
pub fn uplink(s: &Switch, u: &Uplink) -> Vec<Issued> {
    let mut errors = Vec::new();
    let n = u.interfaces.len();
    if n == 0 || n > MAX_UPLINK_INTERFACES {
        errors.push(
            qvsmsg::UPLINK_INTERFACES.with(&[&MAX_UPLINK_INTERFACES.to_string(), &n.to_string()]),
        );
    }
    for (i, at) in u.interfaces.iter().enumerate() {
        if u.interfaces[..i].contains(at) {
            errors.push(qvsmsg::INTERFACE_TWICE.with(&[at]));
        }
    }
    if n > 1 && u.bond.is_none() {
        errors.push(qvsmsg::BOND_MODE_NEEDED.with(&[&n.to_string()]));
    }
    if let Some(native) = u.native_vlan.filter(|v| !s.carries(*v)) {
        errors.push(qvsmsg::NATIVE_VLAN_INVALID.with(&[&native.to_string(), &s.name]));
    }
    errors
}

/// The errors of the limits.
pub fn limits(limits: &Limits) -> Vec<Issued> {
    limits
        .values()
        .iter()
        .zip(LIMIT_RANGES)
        .filter(|((_, v), (_, lo, hi))| !(*lo..=*hi).contains(v))
        .map(|((name, v), (_, lo, hi))| {
            qvsmsg::LIMIT_INVALID.with(&[name, &v.to_string(), &format!("{lo} to {hi}")])
        })
        .collect()
}

/// Every error of the file as a whole.
pub fn switches(all: &Switches) -> Vec<Issued> {
    let mut errors = limits(&all.limits);
    if all.switches.len() > all.limits.switches as usize {
        errors.push(qvsmsg::SWITCHES_TOO_MANY.with(&[
            &all.switches.len().to_string(),
            &all.limits.switches.to_string(),
        ]));
    }
    for (i, s) in all.switches.iter().enumerate() {
        errors.extend(switch(s, &all.limits));
        let before = &all.switches[..i];
        if before.iter().any(|o| o.name == s.name) {
            errors.push(qvsmsg::SWITCH_EXISTS.with(&[&s.name]));
        }
        if s.id == 0 || before.iter().any(|o| o.id == s.id) {
            errors.push(qvsmsg::SWITCH_ID_TWICE.with(&[&s.id.to_string()]));
        }
        for at in s.uplink.iter().flat_map(|u| &u.interfaces) {
            if let Some(o) = before
                .iter()
                .find(|o| o.uplink.as_ref().is_some_and(|u| u.interfaces.contains(at)))
            {
                errors.push(qvsmsg::INTERFACE_TAKEN.with(&[at, &o.name]));
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switch::Bond;

    fn ids(errors: &[Issued]) -> Vec<&str> {
        errors.iter().map(|e| e.id.as_str()).collect()
    }

    fn sw(name: &str, id: u32, vlans: &[u16]) -> Switch {
        Switch {
            name: name.into(),
            id,
            vlans: vlans.to_vec(),
            mtu: 1500,
            uplink: None,
        }
    }

    #[test]
    fn a_good_file_has_no_error() {
        let mut a = sw("PRODNET", 1, &[10, 20]);
        a.uplink = Some(Uplink {
            interfaces: vec!["P1".into(), "P2".into()],
            bond: Some(Bond::Lacp),
            native_vlan: Some(10),
        });
        let all = Switches {
            limits: Limits::default(),
            switches: vec![a, sw("INTERNAL", 2, &[1])],
        };
        assert_eq!(switches(&all), []);
    }

    #[test]
    fn every_error_of_a_switch_is_given_at_once() {
        let mut bad = sw("prod net", 1, &[0, 10, 10, 4095]);
        bad.mtu = 9216;
        bad.uplink = Some(Uplink {
            interfaces: vec!["P1".into(), "P1".into()],
            bond: None,
            native_vlan: Some(30),
        });
        let got = switch(&bad, &Limits::default());
        assert_eq!(
            ids(&got),
            [
                qvsmsg::SWITCH_NAME_INVALID.id,
                qvsmsg::SWITCH_VLAN_INVALID.id,
                qvsmsg::SWITCH_VLAN_INVALID.id,
                qvsmsg::SWITCH_VLAN_INVALID.id,
                qvsmsg::SWITCH_MTU_INVALID.id,
                qvsmsg::INTERFACE_TWICE.id,
                qvsmsg::BOND_MODE_NEEDED.id,
                qvsmsg::NATIVE_VLAN_INVALID.id,
            ]
        );
        let tight = Limits {
            vlans_per_switch: 1,
            ..Limits::default()
        };
        assert_eq!(
            ids(&switch(&sw("A", 1, &[1, 2]), &tight)),
            [qvsmsg::SWITCH_VLANS_TOO_MANY.id]
        );
        let none = Switch {
            uplink: Some(Uplink::default()),
            ..sw("A", 1, &[1])
        };
        assert_eq!(
            ids(&switch(&none, &Limits::default())),
            [qvsmsg::UPLINK_INTERFACES.id]
        );
    }

    #[test]
    fn two_switches_share_no_name_number_or_interface() {
        let mut a = sw("A", 1, &[1]);
        a.uplink = Some(Uplink {
            interfaces: vec!["P1".into()],
            ..Uplink::default()
        });
        let b = Switch {
            name: "A".into(),
            ..a.clone()
        };
        let all = Switches {
            limits: Limits {
                switches: 1,
                ..Limits::default()
            },
            switches: vec![a, b],
        };
        assert_eq!(
            ids(&switches(&all)),
            [
                qvsmsg::SWITCHES_TOO_MANY.id,
                qvsmsg::SWITCH_EXISTS.id,
                qvsmsg::SWITCH_ID_TWICE.id,
                qvsmsg::INTERFACE_TAKEN.id,
            ]
        );
        let limits = Limits {
            adapters_per_partition: 9,
            switches: 0,
            ..Limits::default()
        };
        assert_eq!(
            ids(&super::limits(&limits)),
            [qvsmsg::LIMIT_INVALID.id, qvsmsg::LIMIT_INVALID.id]
        );
    }

    #[test]
    fn a_name_is_a_partitions_kind_of_name() {
        for good in ["A", "PROD_NET1", "ABCDEFGHIJ"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "1NET", "net", "ABCDEFGHIJK", "PROD-NET"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
