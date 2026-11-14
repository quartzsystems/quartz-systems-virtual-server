// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What QS/400's `netlink` lacks for the virtual switches, written over
//! its public `codec` and `Socket`: links, bonds, VLAN sub-interfaces,
//! masters, statistics and link events are its own and used as they
//! are (`docs/PHASE6.md` lists what here belongs there).
//!
//! - [`bridge`]: a VLAN-filtering bridge, a port's rules, its VLANs.
//! - [`fdb`]: static forwarding entries and the forwarding table.
//! - [`filter`]: a partition port's ingress filter (tc flower).
//! - [`tap`]: tap devices (Linux).
//! - [`Nl`]: the client.

pub mod bridge;
pub mod consts;
pub mod fdb;
pub mod filter;
#[cfg(target_os = "linux")]
pub mod tap;

pub use bridge::{LinkVlans, PortRules, PortVlan};
pub use fdb::{FdbEntry, Origin};
pub use netlink::route::{BondMode, BondSpec, Link, LinkStats, Rtnl};
pub use netlink::{Error, Result};

use netlink::{Bus, Socket};

/// The client: QS/400's route client for what it has, and a socket of
/// its own for the rest.
pub struct Nl {
    rtnl: Rtnl,
    sock: Socket,
    sysctl: netlink::sysctl::Sysctl,
}

impl Nl {
    /// Open both sockets.
    pub fn open() -> Result<Self> {
        Ok(Self {
            rtnl: Rtnl::open()?,
            sock: Socket::open(Bus::Route)?,
            sysctl: netlink::sysctl::Sysctl::default(),
        })
    }

    /// QS/400's client: links, bonds, VLANs, masters, addresses.
    pub fn rtnl(&mut self) -> &mut Rtnl {
        &mut self.rtnl
    }

    /// Create the bridge `name` (VLAN filtering, no spanning tree, no
    /// VLAN of its own) and return it.
    pub fn create_bridge(&mut self, name: &str, mtu: u32) -> Result<Link> {
        self.sock.ack(&bridge::new_bridge_request(name, mtu))?;
        self.rtnl.link_by_name(name)
    }

    /// Put `index` on the bridge `master`; 0 takes it off.
    pub fn set_master(&mut self, index: u32, master: u32) -> Result<()> {
        self.sock.ack(&bridge::set_master_request(index, master))
    }

    /// Set a port's rules.
    pub fn port_rules(&mut self, index: u32, rules: PortRules) -> Result<()> {
        self.sock.ack(&bridge::port_rules_request(index, rules))
    }

    /// Add or delete a port's VLANs.
    pub fn port_vlans(&mut self, index: u32, vlans: &[PortVlan], add: bool) -> Result<()> {
        if vlans.is_empty() {
            return Ok(());
        }
        self.sock
            .ack(&bridge::port_vlans_request(index, vlans, add)?)
    }

    /// Take VLANs off the bridge device itself.
    pub fn bridge_self_clear(&mut self, index: u32, vlans: &[u16]) -> Result<()> {
        if vlans.is_empty() {
            return Ok(());
        }
        self.sock
            .ack(&bridge::bridge_self_vlans_delete(index, vlans)?)
    }

    /// Every bridge port's and bridge's VLANs.
    pub fn vlans(&mut self) -> Result<Vec<LinkVlans>> {
        self.sock.dump(&bridge::vlans_dump_request(), |m| {
            if m.kind == netlink::route::consts::RTM_NEWLINK {
                bridge::parse_vlans(m.payload)
            } else {
                Ok(None)
            }
        })
    }

    /// Add or delete a static forwarding entry.
    pub fn fdb_static(&mut self, port: u32, mac: [u8; 6], vlan: u16, add: bool) -> Result<()> {
        self.sock.ack(&fdb::static_request(port, mac, vlan, add)?)
    }

    /// The forwarding table of the bridge `bridge`.
    pub fn fdb(&mut self, bridge: u32) -> Result<Vec<FdbEntry>> {
        let mut entries = self.sock.dump(&fdb::dump_request(bridge), |m| {
            if m.kind == netlink::route::consts::RTM_NEWNEIGH {
                fdb::parse(m.payload)
            } else {
                Ok(None)
            }
        })?;
        entries.retain(|e| e.master == bridge || e.master == 0);
        Ok(entries)
    }

    /// Give `ifindex` the ingress filter of a partition's port that
    /// passes the tagged VLANs `tagged`.
    pub fn ingress_filter(&mut self, ifindex: u32, tagged: &[u16]) -> Result<()> {
        match self.sock.ack(&filter::clsact_request(ifindex)) {
            Ok(()) => {}
            Err(e) if e.is_exists() => {}
            Err(e) => return Err(e),
        }
        for rule in filter::partition_rules(tagged) {
            self.sock.ack(&filter::rule_request(ifindex, &rule))?;
        }
        Ok(())
    }

    /// Turn IPv6 off on the link `name`, so the host never has an
    /// address on it, link-local included. Done before it is brought up.
    pub fn disable_ipv6(&self, name: &str) -> Result<()> {
        self.sysctl
            .write(&format!("ipv6/conf/{name}/disable_ipv6"), "1")
    }

    /// Whether IPv6 is off on `name`.
    pub fn ipv6_disabled(&self, name: &str) -> bool {
        self.sysctl
            .read(&format!("ipv6/conf/{name}/disable_ipv6"))
            .is_ok_and(|v| v == "1")
    }
}

/// `xx:xx:xx:xx:xx:xx`.
pub fn mac_text(mac: &[u8]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// From `xx:xx:xx:xx:xx:xx`.
pub fn mac_parse(text: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = text.split(':');
    for byte in &mut mac {
        let part = parts.next()?;
        if part.len() != 2 {
            return None;
        }
        *byte = u8::from_str_radix(part, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_address_reads_and_writes_as_six_pairs() {
        let mac = mac_parse("02:51:56:0A:00:FF").unwrap();
        assert_eq!(mac, [0x02, 0x51, 0x56, 0x0A, 0x00, 0xFF]);
        assert_eq!(mac_text(&mac), "02:51:56:0a:00:ff");
        for bad in [
            "",
            "02:51:56:0a:00",
            "02:51:56:0a:00:ff:01",
            "2:51:56:0a:00:ff",
            "zz:51:56:0a:00:ff",
        ] {
            assert_eq!(mac_parse(bad), None, "{bad}");
        }
    }
}
