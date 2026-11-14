// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A kernel and a host for the core's tests: links with their masters,
//! each port's VLANs, rules and filter, the forwarding table, IPv6 per
//! link; and the host's interfaces, its management device, the
//! partition manager's adapters, and what was posted and kept.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;

use nl::{FdbEntry, LinkVlans, Origin, PortRules, PortVlan};
use refcode::RefCode;
use vswitch::port::PortSpec;
use vswitch::{Bond, Switches};

use super::Env;
use crate::kernel::{KLink, Kernel};
use crate::proto::{Adapter, Nic};

/// The kernel.
#[derive(Default)]
pub struct FakeKernel {
    pub links: Vec<KLink>,
    next: u32,
    pub vlans: BTreeMap<u32, Vec<PortVlan>>,
    pub fdb: Vec<FdbEntry>,
    pub filters: BTreeMap<u32, Vec<u16>>,
    pub rules: BTreeMap<u32, PortRules>,
    pub ipv6_off: BTreeSet<String>,
    pub max_mtu: BTreeMap<String, u32>,
    pub bonds: BTreeMap<u32, Bond>,
    /// Every MTU set, in order: the link's name and the value.
    pub mtus: Vec<(String, u32)>,
    /// A step that fails: its name.
    pub fail: Option<&'static str>,
}

impl FakeKernel {
    /// A kernel with the physical interfaces `names`.
    pub fn with(names: &[&str]) -> Self {
        let mut k = Self {
            next: 1,
            ..Self::default()
        };
        for n in names {
            k.add(n, None);
            let index = k.index(n).unwrap_or(0);
            if let Some(l) = k.links.iter_mut().find(|l| l.index == index) {
                l.carrier = true;
            }
        }
        k
    }

    fn add(&mut self, name: &str, kind: Option<&str>) -> u32 {
        self.next += 1;
        self.links.push(KLink {
            index: self.next,
            name: name.to_string(),
            kind: kind.map(str::to_string),
            mtu: 1500,
            mac: vec![0x52, 0x54, 0, 0, 0, u8::try_from(self.next).unwrap_or(0)],
            ..KLink::default()
        });
        self.next
    }

    /// A link's index by name.
    pub fn index(&self, name: &str) -> Option<u32> {
        self.links.iter().find(|l| l.name == name).map(|l| l.index)
    }

    /// A link by name.
    pub fn link(&self, name: &str) -> Option<&KLink> {
        self.links.iter().find(|l| l.name == name)
    }

    /// The name of a link's master.
    pub fn master_of(&self, name: &str) -> Option<String> {
        let m = self.link(name)?.master?;
        self.links
            .iter()
            .find(|l| l.index == m)
            .map(|l| l.name.clone())
    }

    /// Set a link's carrier.
    pub fn carrier(&mut self, name: &str, on: bool) {
        if let Some(l) = self.links.iter_mut().find(|l| l.name == name) {
            l.carrier = on;
        }
    }

    /// A tap gone, as when its VMM's file was closed.
    pub fn tap_closed(&mut self, name: &str) {
        if let Some(i) = self.index(name) {
            let _ = self.delete(i);
        }
    }

    fn step(&self, name: &'static str) -> Result<(), String> {
        if self.fail == Some(name) {
            Err(format!("{name} refused"))
        } else {
            Ok(())
        }
    }

    fn get(&mut self, index: u32) -> Result<&mut KLink, String> {
        self.links
            .iter_mut()
            .find(|l| l.index == index)
            .ok_or_else(|| format!("no link {index}"))
    }
}

impl Kernel for FakeKernel {
    fn links(&mut self) -> Result<Vec<KLink>, String> {
        Ok(self.links.clone())
    }

    fn max_mtu(&mut self, index: u32) -> Option<u32> {
        let name = self.links.iter().find(|l| l.index == index)?.name.clone();
        self.max_mtu.get(&name).copied()
    }

    fn create_bridge(&mut self, name: &str, mtu: u32) -> Result<u32, String> {
        self.step("create_bridge")?;
        let i = self.add(name, Some("bridge"));
        self.get(i)?.mtu = mtu;
        Ok(i)
    }

    fn create_bond(&mut self, name: &str, bond: Bond) -> Result<u32, String> {
        self.step("create_bond")?;
        let i = self.add(name, Some("bond"));
        self.bonds.insert(i, bond);
        Ok(i)
    }

    fn create_tap(&mut self, name: &str) -> Result<(u32, File), String> {
        self.step("create_tap")?;
        let i = self.add(name, Some("tun"));
        self.get(i)?.carrier = true;
        let file = File::open("/dev/null").map_err(|e| e.to_string())?;
        Ok((i, file))
    }

    fn delete(&mut self, index: u32) -> Result<(), String> {
        self.step("delete")?;
        let at = self
            .links
            .iter()
            .position(|l| l.index == index)
            .ok_or_else(|| format!("no link {index}"))?;
        self.links.remove(at);
        for l in &mut self.links {
            if l.master == Some(index) {
                l.master = None;
            }
        }
        self.vlans.remove(&index);
        self.fdb.retain(|e| e.port != index);
        self.filters.remove(&index);
        Ok(())
    }

    fn set_master(&mut self, index: u32, master: u32) -> Result<(), String> {
        self.step("set_master")?;
        self.get(index)?.master = (master != 0).then_some(master);
        if master == 0 {
            self.vlans.remove(&index);
            self.fdb.retain(|e| e.port != index);
        }
        Ok(())
    }

    fn enslave(&mut self, index: u32, bond: u32) -> Result<(), String> {
        self.step("enslave")?;
        self.get(index)?.master = Some(bond);
        Ok(())
    }

    fn set_mtu(&mut self, index: u32, mtu: u32) -> Result<(), String> {
        self.step("set_mtu")?;
        let l = self.get(index)?;
        l.mtu = mtu;
        let name = l.name.clone();
        self.mtus.push((name, mtu));
        Ok(())
    }

    fn set_up(&mut self, index: u32, up: bool) -> Result<(), String> {
        self.get(index)?.up = up;
        Ok(())
    }

    fn set_ipv6(&mut self, name: &str, on: bool) -> Result<(), String> {
        if on {
            self.ipv6_off.remove(name);
        } else {
            self.ipv6_off.insert(name.to_string());
        }
        Ok(())
    }

    fn port_rules(&mut self, index: u32, rules: PortRules) -> Result<(), String> {
        self.step("port_rules")?;
        self.rules.insert(index, rules);
        Ok(())
    }

    fn vlans(&mut self) -> Result<Vec<LinkVlans>, String> {
        Ok(self
            .vlans
            .iter()
            .map(|(i, v)| LinkVlans {
                index: *i,
                master: self
                    .links
                    .iter()
                    .find(|l| l.index == *i)
                    .and_then(|l| l.master)
                    .unwrap_or(*i),
                vlans: v.clone(),
            })
            .collect())
    }

    fn set_vlans(&mut self, index: u32, add: &[PortVlan], remove: &[u16]) -> Result<(), String> {
        self.step("set_vlans")?;
        let v = self.vlans.entry(index).or_default();
        v.retain(|x| !remove.contains(&x.vid));
        for a in add {
            v.retain(|x| x.vid != a.vid);
            v.push(*a);
        }
        v.sort();
        Ok(())
    }

    fn clear_bridge_vlans(&mut self, index: u32, vlans: &[u16]) -> Result<(), String> {
        if let Some(v) = self.vlans.get_mut(&index) {
            v.retain(|x| !vlans.contains(&x.vid));
        }
        Ok(())
    }

    fn fdb(&mut self, bridge: u32) -> Result<Vec<FdbEntry>, String> {
        Ok(self
            .fdb
            .iter()
            .filter(|e| e.master == bridge)
            .cloned()
            .collect())
    }

    fn fdb_static(&mut self, port: u32, mac: [u8; 6], vlan: u16, add: bool) -> Result<(), String> {
        self.step("fdb_static")?;
        let master = self.get(port)?.master.unwrap_or(0);
        self.fdb
            .retain(|e| !(e.port == port && e.mac == mac && e.vlan == vlan));
        if add {
            self.fdb.push(FdbEntry {
                port,
                master,
                mac,
                vlan,
                origin: Origin::Static,
            });
        }
        Ok(())
    }

    fn ingress_filter(&mut self, index: u32, tagged: &[u16]) -> Result<(), String> {
        self.step("ingress_filter")?;
        self.filters.insert(index, tagged.to_vec());
        Ok(())
    }
}

/// The host.
#[derive(Default)]
pub struct FakeEnv {
    pub posted: Vec<(String, String)>,
    pub nics: Vec<Nic>,
    pub management: Option<String>,
    pub adapters: Option<Vec<Adapter>>,
    pub saved: Option<Switches>,
    pub ports: Vec<PortSpec>,
    pub refuse_save: bool,
}

impl FakeEnv {
    /// A host with interfaces `(name, location)`, the first the
    /// management one.
    pub fn with(nics: &[(&str, &str)]) -> Self {
        Self {
            nics: nics
                .iter()
                .map(|(n, l)| Nic {
                    name: (*n).to_string(),
                    location: (*l).to_string(),
                    mac: "52:54:00:00:00:01".into(),
                    driver: "virtio_net".into(),
                    speed_mbps: Some(1000),
                    carrier: true,
                })
                .collect(),
            management: nics.first().map(|(n, _)| (*n).to_string()),
            adapters: Some(Vec::new()),
            ..Self::default()
        }
    }

    /// The codes posted.
    pub fn codes(&self) -> Vec<&str> {
        self.posted.iter().map(|(c, _)| c.as_str()).collect()
    }
}

impl Env for FakeEnv {
    fn now(&self) -> u64 {
        1_800_000_000
    }

    fn post(&mut self, code: &'static RefCode, detail: &str) {
        self.posted
            .push((code.code.to_string(), detail.to_string()));
    }

    fn log(&mut self, _line: &str) {}

    fn nics(&mut self) -> Vec<Nic> {
        self.nics.clone()
    }

    fn management(&mut self) -> Option<String> {
        self.management.clone()
    }

    fn adapters(&mut self) -> Result<Vec<Adapter>, String> {
        self.adapters
            .clone()
            .ok_or_else(|| "the partition manager did not answer".to_string())
    }

    fn save(&mut self, switches: &Switches) -> Result<(), String> {
        if self.refuse_save {
            return Err("the state directory is read only".into());
        }
        self.saved = Some(switches.clone());
        Ok(())
    }

    fn save_ports(&mut self, ports: &[PortSpec]) {
        self.ports = ports.to_vec();
    }
}
