// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the switch service asks of the kernel. The service on Linux
//! answers with netlink ([`linux`]); a test answers with a model of the
//! kernel's links.

use std::fs::File;

use nl::{FdbEntry, LinkStats, LinkVlans, PortRules, PortVlan};
use vswitch::Bond;

/// One link, as the service needs to see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KLink {
    /// Interface index.
    pub index: u32,
    /// Name.
    pub name: String,
    /// `bridge`, `bond`, `tun`, or none for a physical device.
    pub kind: Option<String>,
    /// Its master's index: a bridge or a bond.
    pub master: Option<u32>,
    /// MTU.
    pub mtu: u32,
    /// Administratively up.
    pub up: bool,
    /// Carrier.
    pub carrier: bool,
    /// Hardware address.
    pub mac: Vec<u8>,
    /// Counters.
    pub stats: LinkStats,
}

/// What the kernel is asked. Each error is the kernel's, in words.
pub trait Kernel {
    /// Every link.
    fn links(&mut self) -> Result<Vec<KLink>, String>;
    /// The largest MTU the link takes, when its driver says.
    fn max_mtu(&mut self, index: u32) -> Option<u32>;
    /// Make a VLAN-filtering bridge with no VLAN of its own; its index.
    fn create_bridge(&mut self, name: &str, mtu: u32) -> Result<u32, String>;
    /// Make a bond; its index.
    fn create_bond(&mut self, name: &str, bond: Bond) -> Result<u32, String>;
    /// Make a tap with virtio-net headers; its index and its only open
    /// file.
    fn create_tap(&mut self, name: &str) -> Result<(u32, File), String>;
    /// Delete a link.
    fn delete(&mut self, index: u32) -> Result<(), String>;
    /// Put the link on a bridge; 0 takes it off whatever it is on.
    fn set_master(&mut self, index: u32, master: u32) -> Result<(), String>;
    /// Put the link in a bond.
    fn enslave(&mut self, index: u32, bond: u32) -> Result<(), String>;
    /// Set the MTU.
    fn set_mtu(&mut self, index: u32, mtu: u32) -> Result<(), String>;
    /// Bring the link up or down.
    fn set_up(&mut self, index: u32, up: bool) -> Result<(), String>;
    /// Turn IPv6 off on it, so the host has no address there, link-local
    /// or other; or on again for an interface given back.
    fn set_ipv6(&mut self, name: &str, on: bool) -> Result<(), String>;
    /// A bridge port's rules.
    fn port_rules(&mut self, index: u32, rules: PortRules) -> Result<(), String>;
    /// Every bridge port's and bridge's VLANs.
    fn vlans(&mut self) -> Result<Vec<LinkVlans>, String>;
    /// Add VLANs to a port, and take others off it.
    fn set_vlans(&mut self, index: u32, add: &[PortVlan], remove: &[u16]) -> Result<(), String>;
    /// Take VLANs off a bridge device itself.
    fn clear_bridge_vlans(&mut self, index: u32, vlans: &[u16]) -> Result<(), String>;
    /// A bridge's forwarding table.
    fn fdb(&mut self, bridge: u32) -> Result<Vec<FdbEntry>, String>;
    /// Add or delete a static forwarding entry.
    fn fdb_static(&mut self, port: u32, mac: [u8; 6], vlan: u16, add: bool) -> Result<(), String>;
    /// A partition port's ingress filter.
    fn ingress_filter(&mut self, index: u32, tagged: &[u16]) -> Result<(), String>;
}

/// The kernel, through netlink.
#[cfg(target_os = "linux")]
pub mod linux {
    use super::{Bond, FdbEntry, File, KLink, Kernel, LinkVlans, PortRules, PortVlan};
    use nl::{BondMode, BondSpec, Nl};

    /// The kernel.
    pub struct Linux {
        nl: Nl,
    }

    impl Linux {
        /// Open the netlink sockets.
        pub fn open() -> Result<Self, String> {
            Ok(Self {
                nl: Nl::open().map_err(|e| e.to_string())?,
            })
        }
    }

    fn text(e: &nl::Error) -> String {
        e.to_string()
    }

    impl Kernel for Linux {
        fn links(&mut self) -> Result<Vec<KLink>, String> {
            let links = self.nl.rtnl().links().map_err(|e| text(&e))?;
            Ok(links
                .into_iter()
                .map(|l| KLink {
                    index: l.index,
                    carrier: l.has_carrier(),
                    up: l.is_up(),
                    kind: l.kind,
                    master: l.master,
                    mtu: l.mtu,
                    mac: l.mac,
                    stats: l.stats.unwrap_or_default(),
                    name: l.name,
                })
                .collect())
        }

        fn max_mtu(&mut self, index: u32) -> Option<u32> {
            // Some drivers say 0: no limit of theirs.
            self.nl.max_mtu(index).ok().flatten().filter(|m| *m > 0)
        }

        fn create_bridge(&mut self, name: &str, mtu: u32) -> Result<u32, String> {
            self.nl
                .create_bridge(name, mtu)
                .map(|l| l.index)
                .map_err(|e| text(&e))
        }

        fn create_bond(&mut self, name: &str, bond: Bond) -> Result<u32, String> {
            let mode = match bond {
                Bond::ActiveBackup => BondMode::ActiveBackup,
                Bond::Lacp => BondMode::Lacp,
            };
            let mut spec = BondSpec::new(mode);
            if bond == Bond::Lacp {
                spec.lacp_fast = Some(true);
                // Layer 3 and 4: one partition's flows spread too.
                spec.xmit_hash = Some(1);
            }
            self.nl
                .rtnl()
                .create_bond(name, &spec)
                .map(|l| l.index)
                .map_err(|e| text(&e))
        }

        fn create_tap(&mut self, name: &str) -> Result<(u32, File), String> {
            let file = nl::tap::create(name).map_err(|e| format!("tap {name}: {e}"))?;
            let link = self.nl.rtnl().link_by_name(name).map_err(|e| text(&e))?;
            Ok((link.index, file))
        }

        fn delete(&mut self, index: u32) -> Result<(), String> {
            self.nl.rtnl().delete_link(index).map_err(|e| text(&e))
        }

        fn set_master(&mut self, index: u32, master: u32) -> Result<(), String> {
            self.nl.set_master(index, master).map_err(|e| text(&e))
        }

        fn enslave(&mut self, index: u32, bond: u32) -> Result<(), String> {
            self.nl.rtnl().enslave(index, bond).map_err(|e| text(&e))
        }

        fn set_mtu(&mut self, index: u32, mtu: u32) -> Result<(), String> {
            self.nl.rtnl().set_mtu(index, mtu).map_err(|e| text(&e))
        }

        fn set_up(&mut self, index: u32, up: bool) -> Result<(), String> {
            self.nl.rtnl().set_up(index, up).map_err(|e| text(&e))
        }

        fn set_ipv6(&mut self, name: &str, on: bool) -> Result<(), String> {
            self.nl.set_ipv6(name, on).map_err(|e| text(&e))
        }

        fn port_rules(&mut self, index: u32, rules: PortRules) -> Result<(), String> {
            self.nl.port_rules(index, rules).map_err(|e| text(&e))
        }

        fn vlans(&mut self) -> Result<Vec<LinkVlans>, String> {
            self.nl.vlans().map_err(|e| text(&e))
        }

        fn set_vlans(
            &mut self,
            index: u32,
            add: &[PortVlan],
            remove: &[u16],
        ) -> Result<(), String> {
            let remove: Vec<PortVlan> = remove.iter().map(|v| PortVlan::tagged(*v)).collect();
            self.nl
                .port_vlans(index, &remove, false)
                .and_then(|()| self.nl.port_vlans(index, add, true))
                .map_err(|e| text(&e))
        }

        fn clear_bridge_vlans(&mut self, index: u32, vlans: &[u16]) -> Result<(), String> {
            self.nl
                .bridge_self_clear(index, vlans)
                .map_err(|e| text(&e))
        }

        fn fdb(&mut self, bridge: u32) -> Result<Vec<FdbEntry>, String> {
            self.nl.fdb(bridge).map_err(|e| text(&e))
        }

        fn fdb_static(
            &mut self,
            port: u32,
            mac: [u8; 6],
            vlan: u16,
            add: bool,
        ) -> Result<(), String> {
            self.nl
                .fdb_static(port, mac, vlan, add)
                .map_err(|e| text(&e))
        }

        fn ingress_filter(&mut self, index: u32, tagged: &[u16]) -> Result<(), String> {
            self.nl.ingress_filter(index, tagged).map_err(|e| text(&e))
        }
    }
}
