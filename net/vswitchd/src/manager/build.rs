// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A switch in the kernel: its bridge, and its uplink on the bridge.
//! What the kernel has already is kept and brought in line; what is
//! missing is made. The host never has an address on any of it.

use nl::{PortRules, PortVlan};
use vswitch::port::uplink_vlans;
use vswitch::switch::{bond_name, bond_of, bridge_of, tap_of};
use vswitch::{Switch, Uplink};

use super::{Core, Env};
use crate::kernel::{KLink, Kernel};

/// The link named `name`.
pub(crate) fn named<'a>(links: &'a [KLink], name: &str) -> Option<&'a KLink> {
    links.iter().find(|l| l.name == name)
}

/// The VLANs of the kernel's VLAN list, as the bridge has them.
fn to_port(v: vswitch::port::VlanOf) -> PortVlan {
    PortVlan {
        vid: v.vid,
        pvid: v.pvid,
        untagged: v.untagged,
    }
}

impl<K: Kernel, E: Env> Core<K, E> {
    /// The kernel name of the interface at `location`, from the
    /// inventory.
    pub(crate) fn nic_name(&mut self, location: &str) -> Option<String> {
        self.env
            .nics()
            .into_iter()
            .find(|n| n.location == location)
            .map(|n| n.name)
    }

    /// Make or bring in line the bridge of `s`; its index.
    pub(crate) fn bridge(&mut self, s: &Switch) -> Result<u32, String> {
        let links = self.links();
        let name = s.bridge();
        let index = if let Some(l) = named(&links, &name) {
            if l.mtu != s.mtu {
                self.kernel.set_mtu(l.index, s.mtu)?;
            }
            l.index
        } else {
            let index = self.kernel.create_bridge(&name, s.mtu)?;
            self.kernel.set_ipv6(&name, false)?;
            index
        };
        // Its own port to the host is in no VLAN: nothing a partition
        // sends goes up to the host.
        let own: Vec<u16> = self
            .kernel
            .vlans()?
            .into_iter()
            .filter(|v| v.index == index && v.master == index)
            .flat_map(|v| v.vlans.into_iter().map(|p| p.vid))
            .collect();
        self.kernel.clear_bridge_vlans(index, &own)?;
        self.kernel.set_ipv6(&name, false)?;
        self.kernel.set_up(index, true)?;
        Ok(index)
    }

    /// Build `s`: its bridge, then its uplink.
    pub(crate) fn build(&mut self, s: &Switch) -> Result<(), String> {
        let bridge = self.bridge(s)?;
        self.uplink(s, bridge)
    }

    /// Give an interface back: off its bridge or bond, IPv6 as before.
    fn give_back(&mut self, link: &KLink) {
        let _ = self.kernel.set_master(link.index, 0);
        let _ = self.kernel.set_ipv6(&link.name, true);
    }

    /// Put `s`'s uplink on its bridge, as state says, and take off it
    /// what is not that. A missing interface is posted and the rest go
    /// on; a switch with none of its interfaces is internal until they
    /// come.
    pub(crate) fn uplink(&mut self, s: &Switch, bridge: u32) -> Result<(), String> {
        let links = self.links();
        let bond = bond_name(s.id);
        let mut port = None;
        let mut members = Vec::new();
        if let Some(u) = &s.uplink {
            // The management interface is never taken, whatever the state
            // file says.
            let management = self.env.management();
            for location in &u.interfaces {
                match self
                    .nic_name(location)
                    .and_then(|n| named(&links, &n).cloned())
                {
                    Some(l) if management.as_deref() == Some(l.name.as_str()) => self.env.post(
                        &refcode::UPLINK_MISSING,
                        &format!("{}, {location}: it is the management interface", s.name),
                    ),
                    Some(l) => members.push(l),
                    None => self
                        .env
                        .post(&refcode::UPLINK_MISSING, &format!("{}, {location}", s.name)),
                }
            }
            port = self.attach(s, u, bridge, &members)?;
        }
        // Nothing but its partitions' taps and its uplink is on it.
        let links = self.links();
        let keep = port;
        for l in links.iter().filter(|l| l.master == Some(bridge)) {
            if Some(l.index) != keep && tap_of(&l.name).is_none() {
                self.give_back(l);
            }
        }
        // A bond that is not the uplink now goes.
        let bonded = s.uplink.as_ref().is_some_and(|u| u.interfaces.len() > 1);
        if !bonded && let Some(b) = named(&links, &bond).cloned() {
            for m in links.iter().filter(|l| l.master == Some(b.index)) {
                self.give_back(m);
            }
            self.kernel.delete(b.index)?;
        }
        Ok(())
    }

    /// The uplink's interfaces on the bridge, through a bond when there
    /// are several. The port that is on the bridge.
    fn attach(
        &mut self,
        s: &Switch,
        u: &Uplink,
        bridge: u32,
        members: &[KLink],
    ) -> Result<Option<u32>, String> {
        if members.is_empty() {
            return Ok(None);
        }
        let port = if u.interfaces.len() > 1 {
            let links = self.links();
            let name = bond_name(s.id);
            let bond = if let Some(b) = named(&links, &name) {
                b.index
            } else {
                let index = self.kernel.create_bond(&name, u.bond.unwrap_or_default())?;
                self.kernel.set_ipv6(&name, false)?;
                index
            };
            for m in members {
                self.kernel.set_ipv6(&m.name, false)?;
                if m.mtu < s.mtu {
                    self.kernel.set_mtu(m.index, s.mtu)?;
                }
                if m.master != Some(bond) {
                    if m.master.is_some() {
                        self.kernel.set_master(m.index, 0)?;
                    }
                    self.kernel.enslave(m.index, bond)?;
                }
            }
            // A member that is not one now leaves.
            let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
            for l in links.iter().filter(|l| l.master == Some(bond)) {
                if !names.contains(&l.name.as_str()) {
                    self.give_back(l);
                }
            }
            bond
        } else {
            let m = &members[0];
            self.kernel.set_ipv6(&m.name, false)?;
            m.index
        };
        let links = self.links();
        let at = links.iter().find(|l| l.index == port).cloned();
        if let Some(l) = &at {
            if l.mtu < s.mtu {
                self.kernel.set_mtu(port, s.mtu)?;
            }
            if l.master != Some(bridge) {
                if l.master.is_some() {
                    self.kernel.set_master(port, 0)?;
                }
                self.kernel.set_master(port, bridge)?;
            }
        }
        self.kernel.port_rules(port, PortRules::UPLINK)?;
        self.uplink_vlans(s, port)?;
        for m in members {
            self.kernel.set_up(m.index, true)?;
        }
        self.kernel.set_up(port, true)?;
        Ok(Some(port))
    }

    /// The uplink port's VLANs as `s` says: every one tagged, the native
    /// one untagged.
    pub(crate) fn uplink_vlans(&mut self, s: &Switch, port: u32) -> Result<(), String> {
        let Some(u) = &s.uplink else { return Ok(()) };
        let want: Vec<PortVlan> = uplink_vlans(s, u).into_iter().map(to_port).collect();
        let have: Vec<PortVlan> = self
            .kernel
            .vlans()?
            .into_iter()
            .filter(|v| v.index == port)
            .flat_map(|v| v.vlans)
            .collect();
        let remove: Vec<u16> = have
            .iter()
            .filter(|h| !want.contains(h))
            .map(|h| h.vid)
            .collect();
        let add: Vec<PortVlan> = want.into_iter().filter(|w| !have.contains(w)).collect();
        self.kernel.set_vlans(port, &add, &remove)
    }

    /// The uplink port of `s` as the kernel has it now.
    pub(crate) fn uplink_port(&mut self, s: &Switch) -> Option<u32> {
        let links = self.links();
        let bridge = named(&links, &s.bridge())?.index;
        links
            .iter()
            .find(|l| l.master == Some(bridge) && tap_of(&l.name).is_none())
            .map(|l| l.index)
    }

    /// Take away what this service made and state no longer has: a
    /// bridge or bond of no switch, and a tap on no bridge.
    pub(crate) fn sweep(&mut self) {
        let links = self.links();
        for l in &links {
            let stray_bridge = bridge_of(&l.name).is_some_and(|id| self.state.by_id(id).is_none());
            let stray_bond = bond_of(&l.name).is_some_and(|id| {
                self.state
                    .by_id(id)
                    .is_none_or(|s| s.uplink.as_ref().is_none_or(|u| u.interfaces.len() < 2))
            });
            if stray_bond {
                for m in links.iter().filter(|m| m.master == Some(l.index)) {
                    self.give_back(m);
                }
            }
            let lone_tap = tap_of(&l.name).is_some() && l.master.is_none();
            if (stray_bridge || stray_bond || lone_tap)
                && let Err(e) = self.kernel.delete(l.index)
            {
                self.env.log(&format!("{}: not deleted: {e}", l.name));
            }
        }
    }
}
