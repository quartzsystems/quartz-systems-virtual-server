// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Bridges: a VLAN-filtering bridge with spanning tree off and no VLAN
//! of its own, a port's rules, and a port's VLAN membership.

#[allow(clippy::wildcard_imports)] // the numbers module
use crate::consts::*;
use netlink::codec::{Attrs, Builder, NLM_F_CREATE, NLM_F_DUMP, NLM_F_EXCL, NLM_F_REQUEST, ne_u32};
use netlink::route::consts::{
    IFINFOMSG_LEN, IFLA_IFNAME, IFLA_INFO_DATA, IFLA_INFO_KIND, IFLA_LINKINFO, IFLA_MASTER,
    IFLA_MTU, RTM_DELLINK, RTM_GETLINK, RTM_NEWLINK,
};
use netlink::route::link::ifinfomsg;
use netlink::{Error, Result};

/// `ifinfomsg` of `AF_BRIDGE` for the link `index`.
fn bridge_ifinfomsg(index: u32) -> [u8; IFINFOMSG_LEN] {
    let mut b = ifinfomsg(index, 0, 0);
    b[0] = AF_BRIDGE;
    b
}

/// Create the bridge `name` with `mtu`: VLAN filtering on, spanning
/// tree off, no default port VLAN (so neither a port nor the bridge
/// itself is in a VLAN it was not put in), multicast snooping off.
pub fn new_bridge_request(name: &str, mtu: u32) -> Builder {
    let mut b = Builder::new(RTM_NEWLINK, NLM_F_REQUEST | NLM_F_CREATE | NLM_F_EXCL);
    b.bytes(&ifinfomsg(0, 0, 0))
        .str(IFLA_IFNAME, name)
        .u32(IFLA_MTU, mtu);
    b.nest(IFLA_LINKINFO, |b| {
        b.str(IFLA_INFO_KIND, "bridge");
        b.nest(IFLA_INFO_DATA, |b| {
            b.u32(IFLA_BR_STP_STATE, 0);
            b.u32(IFLA_BR_FORWARD_DELAY, 0);
            b.u8(IFLA_BR_VLAN_FILTERING, 1);
            b.u16(IFLA_BR_VLAN_DEFAULT_PVID, 0);
            b.u8(IFLA_BR_MCAST_SNOOPING, 0);
        });
    });
    b
}

/// Put the link `index` on the bridge `master`, or take it off with 0.
pub fn set_master_request(index: u32, master: u32) -> Builder {
    let mut b = Builder::new(RTM_NEWLINK, NLM_F_REQUEST);
    b.bytes(&ifinfomsg(index, 0, 0)).u32(IFLA_MASTER, master);
    b
}

/// The rules of a bridge port.
// Each flag is a port attribute of its own.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortRules {
    /// Learn source addresses.
    pub learning: bool,
    /// Pass only frames whose source has a forwarding entry on the port.
    pub locked: bool,
    /// Drop BPDUs (effective with spanning tree on).
    pub guard: bool,
    /// Flood unknown unicast to the port.
    pub unicast_flood: bool,
    /// Flood multicast to the port.
    pub multicast_flood: bool,
    /// Flood broadcast to the port.
    pub broadcast_flood: bool,
}

impl PortRules {
    /// A partition's port: no learning, locked to its static entries,
    /// BPDUs guarded, no unknown unicast for it.
    pub const PARTITION: PortRules = PortRules {
        learning: false,
        locked: true,
        guard: true,
        unicast_flood: false,
        multicast_flood: true,
        broadcast_flood: true,
    };

    /// An uplink: the outside's addresses are learned, everything
    /// floods to it.
    pub const UPLINK: PortRules = PortRules {
        learning: true,
        locked: false,
        guard: false,
        unicast_flood: true,
        multicast_flood: true,
        broadcast_flood: true,
    };
}

/// Set the port `index`'s rules.
pub fn port_rules_request(index: u32, rules: PortRules) -> Builder {
    let mut b = Builder::new(RTM_SETLINK, NLM_F_REQUEST);
    b.bytes(&bridge_ifinfomsg(index));
    b.nest(IFLA_PROTINFO, |b| {
        b.u8(IFLA_BRPORT_LEARNING, u8::from(rules.learning));
        b.u8(IFLA_BRPORT_LOCKED, u8::from(rules.locked));
        b.u8(IFLA_BRPORT_GUARD, u8::from(rules.guard));
        b.u8(IFLA_BRPORT_UNICAST_FLOOD, u8::from(rules.unicast_flood));
        b.u8(IFLA_BRPORT_MCAST_FLOOD, u8::from(rules.multicast_flood));
        b.u8(IFLA_BRPORT_BCAST_FLOOD, u8::from(rules.broadcast_flood));
    });
    b
}

/// One VLAN of a port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortVlan {
    /// The VLAN ID.
    pub vid: u16,
    /// Untagged frames in are in this VLAN.
    pub pvid: bool,
    /// Frames out in this VLAN leave untagged.
    pub untagged: bool,
}

impl PortVlan {
    /// A VLAN carried tagged.
    pub fn tagged(vid: u16) -> Self {
        Self {
            vid,
            pvid: false,
            untagged: false,
        }
    }

    /// The port's VLAN: untagged in and out.
    pub fn native(vid: u16) -> Self {
        Self {
            vid,
            pvid: true,
            untagged: true,
        }
    }

    fn flags(self) -> u16 {
        let mut f = 0;
        if self.pvid {
            f |= BRIDGE_VLAN_INFO_PVID;
        }
        if self.untagged {
            f |= BRIDGE_VLAN_INFO_UNTAGGED;
        }
        f
    }
}

fn vlan_info(v: PortVlan) -> [u8; 4] {
    let mut b = [0u8; 4];
    b[0..2].copy_from_slice(&v.flags().to_ne_bytes());
    b[2..4].copy_from_slice(&v.vid.to_ne_bytes());
    b
}

fn check(vlans: &[PortVlan]) -> Result<()> {
    match vlans.iter().find(|v| !(1..=4094).contains(&v.vid)) {
        Some(v) => Err(Error::Invalid(format!("VLAN id {}", v.vid))),
        None => Ok(()),
    }
}

/// Add `vlans` to the port `index` (`RTM_SETLINK`), or delete them
/// (`RTM_DELLINK`). A VLAN added again takes the flags given.
pub fn port_vlans_request(index: u32, vlans: &[PortVlan], add: bool) -> Result<Builder> {
    check(vlans)?;
    let kind = if add { RTM_SETLINK } else { RTM_DELLINK };
    let mut b = Builder::new(kind, NLM_F_REQUEST);
    b.bytes(&bridge_ifinfomsg(index));
    b.nest(IFLA_AF_SPEC, |b| {
        b.u16(IFLA_BRIDGE_FLAGS, BRIDGE_FLAGS_MASTER);
        for v in vlans {
            b.attr(IFLA_BRIDGE_VLAN_INFO, &vlan_info(*v));
        }
    });
    Ok(b)
}

/// Take `vlans` off the bridge device itself (its own port to the host).
pub fn bridge_self_vlans_delete(index: u32, vlans: &[u16]) -> Result<Builder> {
    let vlans: Vec<PortVlan> = vlans.iter().map(|v| PortVlan::tagged(*v)).collect();
    check(&vlans)?;
    let mut b = Builder::new(RTM_DELLINK, NLM_F_REQUEST);
    b.bytes(&bridge_ifinfomsg(index));
    b.nest(IFLA_AF_SPEC, |b| {
        b.u16(IFLA_BRIDGE_FLAGS, BRIDGE_FLAGS_SELF);
        for v in &vlans {
            b.attr(IFLA_BRIDGE_VLAN_INFO, &vlan_info(*v));
        }
    });
    Ok(b)
}

/// Dump every bridge port with its VLANs.
pub fn vlans_dump_request() -> Builder {
    let mut b = Builder::new(RTM_GETLINK, NLM_F_REQUEST | NLM_F_DUMP);
    b.bytes(&bridge_ifinfomsg(0))
        .u32(IFLA_EXT_MASK, RTEXT_FILTER_BRVLAN);
    b
}

/// A port's or a bridge's VLANs, as the kernel says them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkVlans {
    /// The link.
    pub index: u32,
    /// Its bridge; its own index for the bridge device itself.
    pub master: u32,
    /// Its VLANs.
    pub vlans: Vec<PortVlan>,
}

/// Parse one reply of [`vlans_dump_request`]. `None` for a link with
/// no bridge.
pub fn parse_vlans(payload: &[u8]) -> Result<Option<LinkVlans>> {
    if payload.len() < IFINFOMSG_LEN {
        return Err(Error::Malformed("ifinfomsg"));
    }
    let index = ne_u32(payload, 4).unwrap_or(0);
    let attrs = Attrs::new(&payload[IFINFOMSG_LEN..]);
    let master = attrs
        .get(IFLA_MASTER)
        .and_then(|a| a.u32())
        .unwrap_or(index);
    let mut vlans = Vec::new();
    if let Some(spec) = attrs.get(IFLA_AF_SPEC) {
        for a in spec.attrs().filter(|a| a.kind == IFLA_BRIDGE_VLAN_INFO) {
            let (Some(flags), Some(vid)) = (a.u16(), netlink::codec::ne_u16(a.data, 2)) else {
                continue;
            };
            vlans.push(PortVlan {
                vid,
                pvid: flags & BRIDGE_VLAN_INFO_PVID != 0,
                untagged: flags & BRIDGE_VLAN_INFO_UNTAGGED != 0,
            });
        }
    }
    Ok(Some(LinkVlans {
        index,
        master,
        vlans,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlink::codec::{NLA_F_NESTED, NLMSG_HDRLEN};

    fn attrs_of(msg: &[u8]) -> Vec<(u16, Vec<u8>)> {
        Attrs::new(&msg[NLMSG_HDRLEN + IFINFOMSG_LEN..])
            .map(|a| (a.kind, a.data.to_vec()))
            .collect()
    }

    #[test]
    fn a_bridge_filters_vlans_with_spanning_tree_off_and_no_default_vlan() {
        let msg = new_bridge_request("qvbr1", 9000).finish(1);
        assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_NEWLINK);
        let attrs = attrs_of(&msg);
        assert_eq!(attrs[0], (IFLA_IFNAME, b"qvbr1\0".to_vec()));
        assert_eq!(attrs[1], (IFLA_MTU, 9000u32.to_ne_bytes().to_vec()));
        let info = Attrs::new(&attrs[2].1);
        assert_eq!(info.get(IFLA_INFO_KIND).unwrap().str(), "bridge");
        let data = info.get(IFLA_INFO_DATA).unwrap().attrs();
        assert_eq!(data.get(IFLA_BR_VLAN_FILTERING).unwrap().u8(), Some(1));
        assert_eq!(data.get(IFLA_BR_STP_STATE).unwrap().u32(), Some(0));
        assert_eq!(data.get(IFLA_BR_VLAN_DEFAULT_PVID).unwrap().u16(), Some(0));
    }

    #[test]
    fn a_partition_port_is_locked_with_no_learning_and_no_unicast_flood() {
        let msg = port_rules_request(7, PortRules::PARTITION).finish(2);
        assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_SETLINK);
        assert_eq!(msg[NLMSG_HDRLEN], AF_BRIDGE);
        let attrs = attrs_of(&msg);
        let raw = u16::from_ne_bytes([
            msg[NLMSG_HDRLEN + IFINFOMSG_LEN + 2],
            msg[NLMSG_HDRLEN + IFINFOMSG_LEN + 3],
        ]);
        assert_eq!(raw, IFLA_PROTINFO | NLA_F_NESTED);
        let port = Attrs::new(&attrs[0].1);
        assert_eq!(port.get(IFLA_BRPORT_LEARNING).unwrap().u8(), Some(0));
        assert_eq!(port.get(IFLA_BRPORT_LOCKED).unwrap().u8(), Some(1));
        assert_eq!(port.get(IFLA_BRPORT_UNICAST_FLOOD).unwrap().u8(), Some(0));
        assert_eq!(port.get(IFLA_BRPORT_BCAST_FLOOD).unwrap().u8(), Some(1));
    }

    #[test]
    fn port_vlans_carry_their_flags_and_parse_back() {
        let vlans = [PortVlan::native(10), PortVlan::tagged(20)];
        let msg = port_vlans_request(7, &vlans, true).unwrap().finish(3);
        let attrs = attrs_of(&msg);
        assert_eq!(attrs[0].0, IFLA_AF_SPEC);
        // The request's own layout is what a dump answers with.
        let mut reply = ifinfomsg(7, 0, 0).to_vec();
        let mut b = Builder::new(0, 0);
        b.u32(IFLA_MASTER, 4);
        b.nest(IFLA_AF_SPEC, |b| {
            for v in &vlans {
                b.attr(IFLA_BRIDGE_VLAN_INFO, &vlan_info(*v));
            }
        });
        reply.extend_from_slice(&b.finish(0)[NLMSG_HDRLEN..]);
        let parsed = parse_vlans(&reply).unwrap().unwrap();
        assert_eq!(parsed.master, 4);
        assert_eq!(parsed.vlans, vlans);
        assert!(port_vlans_request(7, &[PortVlan::tagged(4095)], true).is_err());
        assert!(port_vlans_request(7, &[PortVlan::tagged(0)], false).is_err());
    }
}
