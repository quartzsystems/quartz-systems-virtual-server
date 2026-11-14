// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's port: a tap on its switch's bridge, locked to the
//! adapter's address, in its VLANs, with the ingress filter, handed to
//! the partition manager open. The tap goes when the last open file of
//! it does, so a VMM that ends takes its port with it; the service
//! forgets a port whose tap is gone.

use std::fs::File;

use nl::{PortRules, PortVlan};
use qvsmsg::Issued;
use vswitch::mac;
use vswitch::port::PortSpec;
use vswitch::switch::{tap_name, tap_of};

use super::build::named;
use super::{Core, Env, Port};
use crate::kernel::Kernel;
use crate::proto::{Event, Given};

impl<K: Kernel, E: Env> Core<K, E> {
    /// Every reason `spec` cannot be given a port, each a message.
    fn port_errors(&self, spec: &PortSpec) -> Vec<Issued> {
        let slot = spec.slot.to_string();
        let Some(s) = self.state.get(&spec.switch) else {
            return vec![qvsmsg::ADAPTER_SWITCH_MISSING.with(&[&slot, &spec.switch])];
        };
        let mut errors = Vec::new();
        if let Some(why) = self.failures.get(&s.id) {
            errors.push(qvsmsg::PORT_NOT_MADE.with(&[
                &spec.partition.to_string(),
                &slot,
                &s.name,
                &format!("the switch was not built: {why}"),
            ]));
        }
        for v in spec.missing_on(s) {
            errors.push(qvsmsg::ADAPTER_VLAN_MISSING.with(&[&slot, &s.name, &v.to_string()]));
        }
        if mac::is_group(&spec.mac) || mac::is_zero(&spec.mac) {
            errors.push(qvsmsg::MAC_INVALID.with(&[&slot, &mac::text(&spec.mac)]));
        }
        let key = (spec.partition, spec.slot);
        for (k, p) in &self.ports {
            if *k != key && p.spec.mac == spec.mac {
                errors.push(qvsmsg::PORT_MAC_IN_USE.with(&[
                    &mac::text(&spec.mac),
                    &k.0.to_string(),
                    &k.1.to_string(),
                ]));
            }
        }
        let on_switch = self
            .ports
            .iter()
            .filter(|(k, p)| **k != key && p.switch == s.id)
            .count();
        let limits = self.state.limits;
        if on_switch >= limits.ports_per_switch as usize {
            errors.push(qvsmsg::SWITCH_PORTS_TOO_MANY.with(&[
                &s.name,
                &(on_switch + 1).to_string(),
                &limits.ports_per_switch.to_string(),
            ]));
        }
        let of_partition = self
            .ports
            .keys()
            .filter(|k| **k != key && k.0 == spec.partition)
            .count();
        if of_partition >= limits.adapters_per_partition as usize {
            errors.push(qvsmsg::ETHERNET_TOO_MANY.with(&[
                &(of_partition + 1).to_string(),
                &limits.adapters_per_partition.to_string(),
            ]));
        }
        errors
    }

    /// Open the port `spec`: its tap made and put on the switch with its
    /// rules. What it is, and the tap's only open file.
    pub fn open_port(&mut self, spec: &PortSpec) -> Result<(Given, File), Vec<Issued>> {
        let errors = self.port_errors(spec);
        if !errors.is_empty() {
            return Err(errors);
        }
        let Some(s) = self.state.get(&spec.switch).cloned() else {
            return Err(vec![]);
        };
        // A slot opened again is a new port.
        self.close_port(spec.partition, spec.slot);
        let tap = tap_name(spec.partition, spec.slot);
        let made = self.make_port(spec, &s, &tap);
        match made {
            Ok(file) => {
                self.ports.insert(
                    (spec.partition, spec.slot),
                    Port {
                        spec: spec.clone(),
                        switch: s.id,
                        tap: tap.clone(),
                    },
                );
                self.keep_ports();
                let time = self.env.now();
                self.publish(&Event::Port {
                    partition: spec.partition,
                    slot: spec.slot,
                    switch: s.name.clone(),
                    open: true,
                    time,
                });
                Ok((
                    Given {
                        spec: spec.clone(),
                        tap,
                        mtu: s.mtu,
                    },
                    file,
                ))
            }
            Err(e) => Err(vec![qvsmsg::PORT_NOT_MADE.with(&[
                &spec.partition.to_string(),
                &spec.slot.to_string(),
                &s.name,
                &e,
            ])]),
        }
    }

    /// The tap of a port, on its bridge with its rules. A step that
    /// fails takes the tap away with it.
    fn make_port(
        &mut self,
        spec: &PortSpec,
        s: &vswitch::Switch,
        tap: &str,
    ) -> Result<File, String> {
        let links = self.links();
        let bridge = named(&links, &s.bridge())
            .map(|l| l.index)
            .ok_or_else(|| format!("its bridge {} is not there", s.bridge()))?;
        let (index, file) = self.kernel.create_tap(tap)?;
        let steps = (|| -> Result<(), String> {
            self.kernel.set_ipv6(tap, false)?;
            self.kernel.set_mtu(index, s.mtu)?;
            self.kernel.set_master(index, bridge)?;
            self.kernel.port_rules(index, PortRules::PARTITION)?;
            let vlans: Vec<PortVlan> = spec
                .vlans()
                .into_iter()
                .map(|v| PortVlan {
                    vid: v.vid,
                    pvid: v.pvid,
                    untagged: v.untagged,
                })
                .collect();
            self.kernel.set_vlans(index, &vlans, &[])?;
            for v in &vlans {
                self.kernel.fdb_static(index, spec.mac, v.vid, true)?;
            }
            let tagged: Vec<u16> = vlans
                .iter()
                .filter(|v| !v.untagged)
                .map(|v| v.vid)
                .collect();
            self.kernel.ingress_filter(index, &tagged)?;
            self.kernel.set_up(index, true)
        })();
        match steps {
            Ok(()) => Ok(file),
            Err(e) => {
                drop(file);
                let _ = self.kernel.delete(index);
                Err(e)
            }
        }
    }

    /// Close the port of `partition` in `slot`, or every one of its with
    /// slot 0. How many were.
    pub fn close_port(&mut self, partition: u32, slot: u32) -> u32 {
        let keys: Vec<(u32, u32)> = self
            .ports
            .keys()
            .filter(|k| k.0 == partition && (slot == 0 || k.1 == slot))
            .copied()
            .collect();
        let links = self.links();
        for key in &keys {
            if let Some(p) = self.ports.remove(key) {
                if let Some(l) = named(&links, &p.tap) {
                    let _ = self.kernel.delete(l.index);
                }
                let name = self
                    .state
                    .by_id(p.switch)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                let time = self.env.now();
                self.publish(&Event::Port {
                    partition: key.0,
                    slot: key.1,
                    switch: name,
                    open: false,
                    time,
                });
            }
        }
        if !keys.is_empty() {
            self.keep_ports();
        }
        u32::try_from(keys.len()).unwrap_or(u32::MAX)
    }

    /// Close every port of a partition not in `running`. How many were.
    pub fn reconcile(&mut self, running: &[u32]) -> u32 {
        let gone: Vec<u32> = self
            .ports
            .keys()
            .map(|k| k.0)
            .filter(|p| !running.contains(p))
            .collect();
        let mut n = 0;
        for p in gone {
            n += self.close_port(p, 0);
        }
        // A tap the kernel has that is no port of a running partition.
        let links = self.links();
        for l in &links {
            if let Some((p, slot)) = tap_of(&l.name)
                && !running.contains(&p)
                && !self.ports.contains_key(&(p, slot))
                && self.kernel.delete(l.index).is_ok()
            {
                n += 1;
            }
        }
        n
    }

    /// Take up the ports whose taps the kernel still has, on a switch
    /// of state: what `kept` says of each, or what the kernel does.
    pub(crate) fn adopt(&mut self, kept: &[PortSpec]) -> usize {
        let links = self.links();
        let vlans = self.kernel.vlans().unwrap_or_default();
        for l in &links {
            let Some((partition, slot)) = tap_of(&l.name) else {
                continue;
            };
            let Some(s) = l
                .master
                .and_then(|m| links.iter().find(|b| b.index == m))
                .and_then(|b| vswitch::switch::bridge_of(&b.name))
                .and_then(|id| self.state.by_id(id))
                .cloned()
            else {
                continue;
            };
            let kept_spec = kept
                .iter()
                .find(|k| k.partition == partition && k.slot == slot && k.switch == s.name)
                .cloned();
            let spec = if let Some(spec) = kept_spec {
                spec
            } else {
                // What the kernel has: the port VLAN, the tagged ones,
                // and the address its static entries are for.
                let mine: Vec<PortVlan> = vlans
                    .iter()
                    .filter(|v| v.index == l.index)
                    .flat_map(|v| v.vlans.clone())
                    .collect();
                let mac = l
                    .master
                    .and_then(|b| self.kernel.fdb(b).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .find(|e| e.port == l.index && e.origin == nl::Origin::Static)
                    .map_or([0; 6], |e| e.mac);
                PortSpec {
                    partition,
                    slot,
                    switch: s.name.clone(),
                    vlan: mine.iter().find(|v| v.pvid).map_or(0, |v| v.vid),
                    tagged: mine.iter().filter(|v| !v.pvid).map(|v| v.vid).collect(),
                    mac,
                }
            };
            self.ports.insert(
                (partition, slot),
                Port {
                    spec,
                    switch: s.id,
                    tap: l.name.clone(),
                },
            );
        }
        self.keep_ports();
        self.ports.len()
    }

    /// Forget the ports whose taps are gone: their VMMs ended.
    pub(crate) fn forget_gone(&mut self) {
        let links = self.links();
        if links.is_empty() {
            return;
        }
        let gone: Vec<(u32, u32)> = self
            .ports
            .iter()
            .filter(|(_, p)| named(&links, &p.tap).is_none())
            .map(|(k, _)| *k)
            .collect();
        for (partition, slot) in gone {
            self.close_port(partition, slot);
        }
    }
}
