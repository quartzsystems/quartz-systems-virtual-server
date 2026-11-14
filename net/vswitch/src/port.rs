// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's port on a switch: what the partition manager asks the
//! switch service for at activation, and the VLANs and address the
//! port then passes. The VLANs of an uplink are here too.

use crate::switch::{Switch, Uplink};

/// A partition adapter's port, as asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortSpec {
    /// The partition.
    pub partition: u32,
    /// The adapter's slot.
    pub slot: u32,
    /// The switch.
    pub switch: String,
    /// The port VLAN: untagged to the guest.
    pub vlan: u16,
    /// The VLANs passed to the guest with their tags.
    pub tagged: Vec<u16>,
    /// The adapter's MAC address.
    pub mac: [u8; 6],
}
ipc::wire_struct!(PortSpec {
    partition,
    slot,
    switch,
    vlan,
    tagged,
    mac
});

/// One VLAN of a port: its ID, whether untagged frames in are in it,
/// and whether frames out of it leave untagged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VlanOf {
    /// The VLAN ID.
    pub vid: u16,
    /// Untagged frames in are in it.
    pub pvid: bool,
    /// Frames out leave untagged.
    pub untagged: bool,
}

impl PortSpec {
    /// The VLANs of the port: its own untagged, the tagged ones tagged.
    pub fn vlans(&self) -> Vec<VlanOf> {
        let mut out = vec![VlanOf {
            vid: self.vlan,
            pvid: true,
            untagged: true,
        }];
        for t in &self.tagged {
            if *t != self.vlan && !out.iter().any(|v| v.vid == *t) {
                out.push(VlanOf {
                    vid: *t,
                    pvid: false,
                    untagged: false,
                });
            }
        }
        out
    }

    /// The VLANs `s` does not carry of those the port wants.
    pub fn missing_on(&self, s: &Switch) -> Vec<u16> {
        self.vlans()
            .iter()
            .map(|v| v.vid)
            .filter(|v| !s.carries(*v))
            .collect()
    }
}

/// The VLANs of `s`'s uplink: every VLAN of the switch tagged, the
/// native one untagged.
pub fn uplink_vlans(s: &Switch, u: &Uplink) -> Vec<VlanOf> {
    s.vlans
        .iter()
        .map(|v| {
            let native = u.native_vlan == Some(*v);
            VlanOf {
                vid: *v,
                pvid: native,
                untagged: native,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_has_its_vlan_untagged_and_the_listed_ones_tagged() {
        let p = PortSpec {
            vlan: 10,
            tagged: vec![20, 10, 30, 20],
            ..PortSpec::default()
        };
        let v = p.vlans();
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].vid, v[0].pvid, v[0].untagged), (10, true, true));
        assert_eq!((v[1].vid, v[1].pvid, v[1].untagged), (20, false, false));
        let s = Switch {
            name: "A".into(),
            id: 1,
            vlans: vec![10, 20],
            mtu: 1500,
            uplink: None,
        };
        assert_eq!(p.missing_on(&s), [30]);
        let u = Uplink {
            interfaces: vec!["P1".into()],
            bond: None,
            native_vlan: Some(20),
        };
        let up = uplink_vlans(&s, &u);
        assert_eq!((up[0].vid, up[0].untagged), (10, false));
        assert_eq!((up[1].vid, up[1].pvid, up[1].untagged), (20, true, true));
    }
}
