// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The socket's requests. One that is refused answers with every
//! message that applies and changes nothing; one that changes a switch
//! has done it in the kernel and written the state file before it
//! answers, and a step the kernel refuses puts back what was.

use qvsmsg::Issued;
use vswitch::switch::tap_of;
use vswitch::{Limits, Switch, Switches, Uplink, validate};

use super::build::named;
use super::{Core, Env};
use crate::kernel::Kernel;
use crate::proto::{Counters, Event, FdbRow, Interface, PortStatus, SwitchStatus, VlanUse};

/// A request's refusal.
pub type Refused = Vec<Issued>;

fn failed(what: &str, e: &str) -> Refused {
    vec![qvsmsg::SWITCH_NOT_CHANGED.with(&[what, e])]
}

impl<K: Kernel, E: Env> Core<K, E> {
    /// A switch's status.
    pub fn status(&mut self, s: &Switch) -> SwitchStatus {
        let members = self.members(s);
        SwitchStatus {
            bridge: s.bridge(),
            uplink_state: Self::uplink_state(s, &members).to_string(),
            members,
            ports: u32::try_from(self.ports.values().filter(|p| p.switch == s.id).count())
                .unwrap_or(u32::MAX),
            failure: self.failures.get(&s.id).cloned().unwrap_or_default(),
            switch: Some(s.clone()),
        }
    }

    /// Every switch's status.
    pub fn statuses(&mut self) -> Vec<SwitchStatus> {
        let switches = self.state.switches.clone();
        switches.iter().map(|s| self.status(s)).collect()
    }

    fn get(&self, name: &str) -> Result<Switch, Refused> {
        self.state
            .get(name)
            .cloned()
            .ok_or_else(|| vec![qvsmsg::SWITCH_MISSING.with(&[name])])
    }

    /// Write `next` as the state, or say why not.
    fn keep(&mut self, next: Switches, what: &str) -> Result<(), Refused> {
        self.env.save(&next).map_err(|e| failed(what, &e))?;
        self.state = next;
        Ok(())
    }

    fn said(&mut self, name: &str, changed: &str) {
        self.env
            .post(&refcode::SWITCH_CHANGED, &format!("{name}: {changed}"));
        let status = self.state.get(name).cloned().map(|s| self.status(&s));
        let time = self.env.now();
        self.publish(&Event::Switch {
            name: name.to_string(),
            status,
            time,
        });
    }

    /// Make an internal switch.
    pub fn create(&mut self, name: &str, vlans: &[u16], mtu: u32) -> Result<SwitchStatus, Refused> {
        let mut vlans = vlans.to_vec();
        vlans.sort_unstable();
        let s = Switch {
            name: name.to_string(),
            id: self.state.free_id(),
            vlans,
            mtu: if mtu == 0 {
                vswitch::switch::DEFAULT_MTU
            } else {
                mtu
            },
            uplink: None,
        };
        let mut errors = validate::switch(&s, &self.state.limits);
        if self.state.get(name).is_some() {
            errors.push(qvsmsg::SWITCH_EXISTS.with(&[name]));
        }
        if self.state.switches.len() >= self.state.limits.switches as usize {
            errors.push(qvsmsg::SWITCHES_TOO_MANY.with(&[
                &(self.state.switches.len() + 1).to_string(),
                &self.state.limits.switches.to_string(),
            ]));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        if let Err(e) = self.bridge(&s) {
            self.sweep_switch(s.id);
            return Err(failed("make the switch's bridge", &e));
        }
        let mut next = self.state.clone();
        next.switches.push(s.clone());
        if let Err(r) = self.keep(next, "keep the switch") {
            self.sweep_switch(s.id);
            return Err(r);
        }
        let vlans: Vec<String> = s.vlans.iter().map(u16::to_string).collect();
        self.env.post(
            &refcode::SWITCH_CREATED,
            &format!("{}, VLANs {}, MTU {}", s.name, vlans.join(" "), s.mtu),
        );
        let status = self.status(&s);
        let time = self.env.now();
        self.publish(&Event::Switch {
            name: s.name.clone(),
            status: Some(status.clone()),
            time,
        });
        Ok(status)
    }

    /// The bridge and bond of a switch number, deleted.
    fn sweep_switch(&mut self, id: u32) {
        let links = self.links();
        for name in [
            vswitch::switch::bridge_name(id),
            vswitch::switch::bond_name(id),
        ] {
            if let Some(l) = named(&links, &name) {
                let _ = self.kernel.delete(l.index);
            }
        }
    }

    /// The profiles' adapters on `switch`, or a refusal when the
    /// partition manager does not say.
    fn adapters_on(&mut self, switch: &str) -> Result<Vec<crate::proto::Adapter>, Refused> {
        let all = self
            .env
            .adapters()
            .map_err(|e| failed("ask the partition manager which profiles use it", &e))?;
        Ok(all.into_iter().filter(|a| a.switch == switch).collect())
    }

    /// The largest MTU of the interfaces at `locations`: each that says,
    /// with its location.
    fn carried(&mut self, locations: &[String]) -> Vec<(String, u32)> {
        let links = self.links();
        let mut out = Vec::new();
        for at in locations {
            let index = self
                .nic_name(at)
                .and_then(|n| named(&links, &n).map(|l| l.index));
            if let Some(max) = index.and_then(|i| self.kernel.max_mtu(i)) {
                out.push((at.clone(), max));
            }
        }
        out
    }

    /// Change a switch's VLANs and MTU, while partitions run on it.
    pub fn change(&mut self, name: &str, vlans: &[u16], mtu: u32) -> Result<(), Refused> {
        let old = self.get(name)?;
        let mut vlans = vlans.to_vec();
        vlans.sort_unstable();
        let next_switch = Switch {
            vlans: vlans.clone(),
            mtu,
            ..old.clone()
        };
        let mut errors = validate::switch(&next_switch, &self.state.limits);
        let removed: Vec<u16> = old
            .vlans
            .iter()
            .filter(|v| !vlans.contains(v))
            .copied()
            .collect();
        if !removed.is_empty() {
            for a in self.adapters_on(name)? {
                for v in std::iter::once(a.vlan).chain(a.tagged.iter().copied()) {
                    if removed.contains(&v) {
                        errors.push(qvsmsg::SWITCH_VLAN_NAMED.with(&[
                            &v.to_string(),
                            name,
                            &a.name,
                            &a.profile,
                            &a.slot.to_string(),
                        ]));
                    }
                }
            }
            for p in self.ports.values().filter(|p| p.switch == old.id) {
                for v in p.spec.vlans() {
                    if removed.contains(&v.vid) {
                        errors.push(qvsmsg::SWITCH_IN_USE.with(&[
                            name,
                            &p.spec.partition.to_string(),
                            &p.spec.slot.to_string(),
                        ]));
                    }
                }
            }
        }
        if mtu != old.mtu
            && let Some(u) = &old.uplink
        {
            for (at, max) in self.carried(&u.interfaces) {
                if max < mtu {
                    errors.push(qvsmsg::MTU_TOO_LARGE.with(&[
                        &at,
                        name,
                        &mtu.to_string(),
                        &max.to_string(),
                    ]));
                }
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let applied = self.apply_change(&old, &next_switch);
        if let Err(e) = applied {
            let _ = self.apply_change(&next_switch, &old);
            return Err(failed("change the switch", &e));
        }
        let mut next = self.state.clone();
        if let Some(s) = next.switches.iter_mut().find(|s| s.name == name) {
            *s = next_switch.clone();
        }
        if let Err(r) = self.keep(next, "keep the switch") {
            let _ = self.apply_change(&next_switch, &old);
            return Err(r);
        }
        let list: Vec<String> = next_switch.vlans.iter().map(u16::to_string).collect();
        self.said(name, &format!("VLANs {}, MTU {}", list.join(" "), mtu));
        Ok(())
    }

    /// The kernel from `old` to `new`: the uplink's VLANs, and the MTU
    /// of the uplink, the bridge and every port.
    fn apply_change(&mut self, old: &Switch, new: &Switch) -> Result<(), String> {
        let links = self.links();
        let bridge = named(&links, &new.bridge())
            .map(|l| l.index)
            .ok_or_else(|| format!("its bridge {} is not there", new.bridge()))?;
        if new.mtu != old.mtu {
            // Larger: the uplink first, so the bridge never has more than
            // it carries. Smaller: the ports first.
            let mut order: Vec<u32> = Vec::new();
            let uplink = self.uplink_port(new);
            let members: Vec<u32> = links
                .iter()
                .filter(|l| uplink.is_some_and(|u| l.master == Some(u)))
                .map(|l| l.index)
                .collect();
            let taps: Vec<u32> = links
                .iter()
                .filter(|l| l.master == Some(bridge) && tap_of(&l.name).is_some())
                .map(|l| l.index)
                .collect();
            let up: Vec<u32> = members.iter().copied().chain(uplink).collect();
            if new.mtu > old.mtu {
                order.extend(&up);
                order.extend(&taps);
                order.push(bridge);
            } else {
                order.extend(&taps);
                order.push(bridge);
                order.extend(up.iter().rev());
            }
            for index in order {
                self.kernel.set_mtu(index, new.mtu)?;
            }
        }
        if let Some(port) = self.uplink_port(new) {
            self.uplink_vlans(new, port)?;
        }
        Ok(())
    }

    /// Delete a switch no partition has a port on. One a profile names
    /// is deleted only when `force`.
    pub fn delete(&mut self, name: &str, force: bool) -> Result<(), Refused> {
        let s = self.get(name)?;
        let mut errors: Refused = self
            .ports
            .values()
            .filter(|p| p.switch == s.id)
            .map(|p| {
                qvsmsg::SWITCH_IN_USE.with(&[
                    name,
                    &p.spec.partition.to_string(),
                    &p.spec.slot.to_string(),
                ])
            })
            .collect();
        if !force {
            for a in self.adapters_on(name)? {
                errors.push(qvsmsg::SWITCH_NAMED.with(&[
                    name,
                    &a.name,
                    &a.profile,
                    &a.slot.to_string(),
                ]));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut next = self.state.clone();
        next.switches.retain(|o| o.name != name);
        self.keep(next, "keep the switches")?;
        // Its uplink goes back, then its bridge goes.
        let internal = Switch {
            uplink: None,
            ..s.clone()
        };
        let links = self.links();
        if let Some(b) = named(&links, &s.bridge()).map(|l| l.index) {
            let _ = self.uplink(&internal, b);
        }
        self.sweep_switch(s.id);
        self.failures.remove(&s.id);
        self.carrier.retain(|(id, _), _| *id != s.id);
        self.env.post(&refcode::SWITCH_DELETED, name);
        let time = self.env.now();
        self.publish(&Event::Switch {
            name: name.to_string(),
            status: None,
            time,
        });
        Ok(())
    }

    /// Rename a switch no profile names. Its kernel objects are named by
    /// its number and are not touched.
    pub fn rename(&mut self, name: &str, to: &str) -> Result<(), Refused> {
        let _ = self.get(name)?;
        let mut errors = Vec::new();
        if !validate::valid_name(to) {
            errors.push(qvsmsg::SWITCH_NAME_INVALID.with(&[to]));
        }
        if self.state.get(to).is_some() {
            errors.push(qvsmsg::SWITCH_EXISTS.with(&[to]));
        }
        for a in self.adapters_on(name)? {
            errors.push(qvsmsg::SWITCH_NAMED.with(&[
                name,
                &a.name,
                &a.profile,
                &a.slot.to_string(),
            ]));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut next = self.state.clone();
        if let Some(s) = next.switches.iter_mut().find(|s| s.name == name) {
            s.name = to.to_string();
        }
        self.keep(next, "keep the switch")?;
        for p in self.ports.values_mut() {
            if p.spec.switch == name {
                p.spec.switch = to.to_string();
            }
        }
        self.keep_ports();
        self.said(to, &format!("renamed from {name}"));
        Ok(())
    }

    /// Every reason `u` cannot be `s`'s uplink, the host looked at.
    fn uplink_errors(&mut self, s: &Switch, u: &Uplink) -> Refused {
        let mut errors = validate::uplink(s, u);
        let nics = self.env.nics();
        let management = self.env.management();
        for at in &u.interfaces {
            let Some(nic) = nics.iter().find(|n| n.location == *at) else {
                errors.push(qvsmsg::INTERFACE_MISSING.with(&[at]));
                continue;
            };
            if management.as_deref() == Some(nic.name.as_str()) {
                errors.push(qvsmsg::INTERFACE_MANAGEMENT.with(&[at]));
            }
            if let Some(o) = self.state.uplink_of(at).filter(|o| o.name != s.name) {
                errors.push(qvsmsg::INTERFACE_TAKEN.with(&[at, &o.name]));
            }
        }
        for (at, max) in self.carried(&u.interfaces) {
            if max < s.mtu {
                errors.push(qvsmsg::MTU_TOO_LARGE.with(&[
                    &at,
                    &s.name,
                    &s.mtu.to_string(),
                    &max.to_string(),
                ]));
            }
        }
        errors
    }

    /// Set a switch's uplink, or move it to other interfaces.
    pub fn set_uplink(&mut self, name: &str, uplink: &Uplink) -> Result<(), Refused> {
        let old = self.get(name)?;
        let mut u = uplink.clone();
        if u.interfaces.len() < 2 {
            u.bond = None;
        }
        let errors = self.uplink_errors(&old, &u);
        if !errors.is_empty() {
            return Err(errors);
        }
        let new = Switch {
            uplink: Some(u.clone()),
            ..old.clone()
        };
        // A bond of another mode is made again.
        let remake = old
            .uplink
            .as_ref()
            .is_some_and(|o| o.interfaces.len() > 1 && o.bond != u.bond);
        if remake {
            let internal = Switch {
                uplink: None,
                ..old.clone()
            };
            let _ = self.build(&internal);
        }
        if let Err(e) = self.build(&new) {
            let _ = self.build(&old);
            return Err(failed("put the uplink on the switch", &e));
        }
        let mut next = self.state.clone();
        if let Some(s) = next.switches.iter_mut().find(|s| s.name == name) {
            *s = new.clone();
        }
        if let Err(r) = self.keep(next, "keep the switch") {
            let _ = self.build(&old);
            return Err(r);
        }
        self.failures.remove(&new.id);
        self.carrier.retain(|(id, _), _| *id != new.id);
        self.watch_quietly_one(&new);
        let mode = u
            .bond
            .map_or(String::new(), |b| format!(" bonded {}", b.word()));
        let native = u
            .native_vlan
            .map_or(String::new(), |v| format!(", native VLAN {v}"));
        self.said(
            name,
            &format!("uplink {}{mode}{native}", u.interfaces.join(" ")),
        );
        Ok(())
    }

    fn watch_quietly_one(&mut self, s: &Switch) {
        for m in self.members(s) {
            self.carrier.insert((s.id, m.location), m.carrier);
        }
    }

    /// Clear a switch's uplink: it is internal from then on.
    pub fn clear_uplink(&mut self, name: &str) -> Result<(), Refused> {
        let old = self.get(name)?;
        if old.uplink.is_none() {
            return Ok(());
        }
        let new = Switch {
            uplink: None,
            ..old.clone()
        };
        if let Err(e) = self.build(&new) {
            let _ = self.build(&old);
            return Err(failed("take the uplink off the switch", &e));
        }
        let mut next = self.state.clone();
        if let Some(s) = next.switches.iter_mut().find(|s| s.name == name) {
            *s = new;
        }
        self.keep(next, "keep the switch")?;
        self.carrier.retain(|(id, _), _| *id != old.id);
        self.said(name, "uplink cleared: internal");
        Ok(())
    }

    /// Every network interface of the host and what it is.
    pub fn interfaces(&mut self) -> Vec<Interface> {
        let nics = self.env.nics();
        let management = self.env.management();
        let links = self.links();
        nics.into_iter()
            .map(|n| {
                let link = named(&links, &n.name);
                let max_mtu = link
                    .map(|l| l.index)
                    .and_then(|i| self.kernel.max_mtu(i))
                    .unwrap_or(0);
                let owner = self.state.uplink_of(&n.location).cloned();
                let is_management = management.as_deref() == Some(n.name.as_str());
                let what = match (&owner, is_management) {
                    (_, true) => "Management".to_string(),
                    (Some(s), false)
                        if s.uplink.as_ref().is_some_and(|u| u.interfaces.len() > 1) =>
                    {
                        format!("Bond member of {}", s.name)
                    }
                    (Some(s), false) => format!("Uplink of {}", s.name),
                    (None, false) => "Not used".to_string(),
                };
                Interface {
                    carrier: link.map_or(n.carrier, |l| l.carrier),
                    speed_mbps: n.speed_mbps.unwrap_or(0),
                    max_mtu,
                    what,
                    switch: owner.map(|s| s.name).unwrap_or_default(),
                    management: is_management,
                    name: n.name,
                    location: n.location,
                    mac: n.mac,
                    driver: n.driver,
                }
            })
            .collect()
    }

    /// The partitions' ports: of `switch`, or every one when empty.
    pub fn port_statuses(&mut self, switch: &str) -> Vec<PortStatus> {
        let links = self.links();
        self.ports
            .values()
            .filter(|p| switch.is_empty() || p.spec.switch == switch)
            .map(|p| {
                let link = named(&links, &p.tap);
                // The tap's receive is what the partition sent.
                let st = link.map(|l| l.stats).unwrap_or_default();
                PortStatus {
                    spec: p.spec.clone(),
                    tap: p.tap.clone(),
                    link: link.is_some_and(|l| l.carrier),
                    counters: Counters {
                        sent_bytes: st.rx_bytes,
                        sent_packets: st.rx_packets,
                        sent_dropped: st.rx_dropped,
                        received_bytes: st.tx_bytes,
                        received_packets: st.tx_packets,
                        received_dropped: st.tx_dropped,
                    },
                }
            })
            .collect()
    }

    /// A switch's forwarding table.
    pub fn fdb(&mut self, name: &str) -> Result<Vec<FdbRow>, Refused> {
        let s = self.get(name)?;
        let links = self.links();
        let Some(bridge) = named(&links, &s.bridge()).map(|l| l.index) else {
            return Ok(Vec::new());
        };
        let entries = self
            .kernel
            .fdb(bridge)
            .map_err(|e| failed("read the forwarding table", &e))?;
        let uplink = self.uplink_port(&s);
        Ok(entries
            .into_iter()
            .map(|e| {
                let link = links.iter().find(|l| l.index == e.port);
                let (port, partition, slot) = match link.and_then(|l| tap_of(&l.name)) {
                    Some((p, slot)) => (format!("partition {p} slot {slot}"), p, slot),
                    None if Some(e.port) == uplink => ("uplink".to_string(), 0, 0),
                    None if e.port == bridge => ("switch".to_string(), 0, 0),
                    None => (link.map(|l| l.name.clone()).unwrap_or_default(), 0, 0),
                };
                FdbRow {
                    mac: vswitch::mac::text(&e.mac),
                    vlan: e.vlan,
                    port,
                    partition,
                    slot,
                    origin: e.origin.word().to_string(),
                }
            })
            .collect())
    }

    /// Who is on which VLAN of every switch.
    pub fn vlan_use(&self) -> Vec<VlanUse> {
        let mut out = Vec::new();
        for s in &self.state.switches {
            let native = s.uplink.as_ref().and_then(|u| u.native_vlan);
            for v in &s.vlans {
                let ports = self
                    .ports
                    .values()
                    .filter(|p| p.switch == s.id)
                    .filter_map(|p| {
                        p.spec
                            .vlans()
                            .into_iter()
                            .find(|x| x.vid == *v)
                            .map(|x| (p.spec.partition, p.spec.slot, !x.untagged))
                    })
                    .collect();
                out.push(VlanUse {
                    switch: s.name.clone(),
                    vlan: *v,
                    native: native == Some(*v),
                    ports,
                });
            }
        }
        out
    }

    /// Change the limits.
    pub fn change_limits(&mut self, limits: Limits) -> Result<(), Refused> {
        let mut next = self.state.clone();
        next.limits = limits;
        let errors = validate::switches(&next);
        if !errors.is_empty() {
            return Err(errors);
        }
        self.keep(next, "keep the limits")
    }
}
