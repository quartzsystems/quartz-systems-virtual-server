// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The switches as the host keeps them: `vswitch.toml` in the state
//! directory, the switches and the limits.
//!
//! ```toml
//! schema_version = 1
//!
//! [limits]
//! switches = 16
//!
//! [[switch]]
//! name = "PRODNET"
//! id = 1
//! vlans = [10, 20]
//! mtu = 1500
//!
//! [switch.uplink]
//! interfaces = ["P0000-B01-D00-F0-E0", "P0000-B01-D00-F0-E1"]
//! bond = "active-backup"
//! native_vlan = 10
//! ```

use serde::{Deserialize, Serialize};
use state::store::{Document, Migration};

/// The least MTU of a switch.
pub const MIN_MTU: u32 = 1280;
/// The greatest.
pub const MAX_MTU: u32 = 9000;
/// A switch's MTU when it is made with none.
pub const DEFAULT_MTU: u32 = 1500;
/// The most interfaces of one uplink.
pub const MAX_UPLINK_INTERFACES: usize = 8;
/// The most tagged VLANs of one adapter.
pub const MAX_TAGGED: usize = 32;

/// How a bond of several interfaces works.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Bond {
    /// One member carries the traffic; another takes over when it fails.
    #[default]
    #[serde(rename = "active-backup")]
    ActiveBackup,
    /// LACP with the physical switch: every member carries traffic.
    #[serde(rename = "802.3ad")]
    Lacp,
}
ipc::wire_enum!(Bond { 0 => ActiveBackup, 1 => Lacp });

impl Bond {
    /// As state and the console have it.
    pub fn word(self) -> &'static str {
        match self {
            Bond::ActiveBackup => "active-backup",
            Bond::Lacp => "802.3ad",
        }
    }

    /// From its word.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "active-backup" => Some(Bond::ActiveBackup),
            "802.3ad" => Some(Bond::Lacp),
            _ => None,
        }
    }
}

/// A switch's uplink: one physical interface, or a bond of several,
/// each named by its location code. A trunk: the switch's VLANs are
/// tagged on it, but the native one.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Uplink {
    /// The interfaces, by location code.
    pub interfaces: Vec<String>,
    /// How several are bonded; one interface has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bond: Option<Bond>,
    /// The VLAN carried untagged on it; none when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_vlan: Option<u16>,
}
ipc::wire_struct!(Uplink {
    interfaces,
    bond,
    native_vlan
});

/// One switch.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Switch {
    /// Its name.
    pub name: String,
    /// Its number, given when it is made and never changed: the kernel's
    /// objects are named by it, so a rename touches none of them.
    pub id: u32,
    /// The VLANs it carries, in order.
    pub vlans: Vec<u16>,
    /// Its MTU.
    pub mtu: u32,
    /// Its uplink; an internal switch has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uplink: Option<Uplink>,
}
ipc::wire_struct!(Switch {
    name,
    id,
    vlans,
    mtu,
    uplink
});

impl Switch {
    /// Whether it carries `vlan`.
    pub fn carries(&self, vlan: u16) -> bool {
        self.vlans.contains(&vlan)
    }

    /// The bridge's name in the kernel.
    pub fn bridge(&self) -> String {
        bridge_name(self.id)
    }

    /// The bond's name in the kernel, when its uplink is a bond.
    pub fn bond(&self) -> Option<String> {
        self.uplink
            .as_ref()
            .filter(|u| u.interfaces.len() > 1)
            .map(|_| bond_name(self.id))
    }
}

/// The bridge of switch `id`.
pub fn bridge_name(id: u32) -> String {
    format!("qvbr{id}")
}

/// The bond of switch `id`'s uplink.
pub fn bond_name(id: u32) -> String {
    format!("qvbo{id}")
}

/// The tap of slot `slot` of partition `partition`.
pub fn tap_name(partition: u32, slot: u32) -> String {
    format!("qvt{partition}s{slot}")
}

/// The partition and slot of a tap's name.
pub fn tap_of(name: &str) -> Option<(u32, u32)> {
    let (p, s) = name.strip_prefix("qvt")?.split_once('s')?;
    Some((p.parse().ok()?, s.parse().ok()?))
}

/// The switch of a bridge's name.
pub fn bridge_of(name: &str) -> Option<u32> {
    name.strip_prefix("qvbr")?.parse().ok()
}

/// The switch of a bond's name.
pub fn bond_of(name: &str) -> Option<u32> {
    name.strip_prefix("qvbo")?.parse().ok()
}

/// The limits, with their defaults.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Switches on the host.
    #[serde(default = "default_switches")]
    pub switches: u32,
    /// VLANs of one switch.
    #[serde(default = "default_vlans")]
    pub vlans_per_switch: u32,
    /// Ethernet adapters of one partition.
    #[serde(default = "default_adapters")]
    pub adapters_per_partition: u32,
    /// Partitions' ports on one switch.
    #[serde(default = "default_ports")]
    pub ports_per_switch: u32,
}
ipc::wire_struct!(Limits {
    switches,
    vlans_per_switch,
    adapters_per_partition,
    ports_per_switch
});

fn default_switches() -> u32 {
    16
}
fn default_vlans() -> u32 {
    256
}
fn default_adapters() -> u32 {
    8
}
fn default_ports() -> u32 {
    256
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            switches: default_switches(),
            vlans_per_switch: default_vlans(),
            adapters_per_partition: default_adapters(),
            ports_per_switch: default_ports(),
        }
    }
}

/// The ranges each limit may be set in: name, least, most.
pub const LIMIT_RANGES: [(&str, u32, u32); 4] = [
    ("switches", 1, 64),
    ("vlans_per_switch", 1, 4094),
    ("adapters_per_partition", 1, 8),
    ("ports_per_switch", 1, 1024),
];

impl Limits {
    /// Each limit by name.
    pub fn values(&self) -> [(&'static str, u32); 4] {
        [
            ("switches", self.switches),
            ("vlans_per_switch", self.vlans_per_switch),
            ("adapters_per_partition", self.adapters_per_partition),
            ("ports_per_switch", self.ports_per_switch),
        ]
    }
}

/// `vswitch.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Switches {
    /// The limits.
    #[serde(default)]
    pub limits: Limits,
    /// The switches, in the order they were made.
    #[serde(default, rename = "switch", skip_serializing_if = "Vec::is_empty")]
    pub switches: Vec<Switch>,
}

impl Switches {
    /// The switch named `name`.
    pub fn get(&self, name: &str) -> Option<&Switch> {
        self.switches.iter().find(|s| s.name == name)
    }

    /// The switch whose number is `id`.
    pub fn by_id(&self, id: u32) -> Option<&Switch> {
        self.switches.iter().find(|s| s.id == id)
    }

    /// The lowest number no switch has.
    pub fn free_id(&self) -> u32 {
        (1..=u32::MAX)
            .find(|id| self.by_id(*id).is_none())
            .unwrap_or(1)
    }

    /// The switch whose uplink has the interface `location`.
    pub fn uplink_of(&self, location: &str) -> Option<&Switch> {
        self.switches.iter().find(|s| {
            s.uplink
                .as_ref()
                .is_some_and(|u| u.interfaces.iter().any(|i| i == location))
        })
    }
}

impl Document for Switches {
    const FILE: &'static str = "vswitch.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        match crate::validate::switches(self).first() {
            Some(e) => Err(e.to_string()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_reads_as_its_doc_says_and_writes_back() {
        let text = r#"
schema_version = 1

[limits]
switches = 4

[[switch]]
name = "PRODNET"
id = 1
vlans = [10, 20]
mtu = 1500

[switch.uplink]
interfaces = ["P0000-B01-D00-F0-E0", "P0000-B01-D00-F0-E1"]
bond = "active-backup"
native_vlan = 10

[[switch]]
name = "BACKEND"
id = 2
vlans = [30]
mtu = 9000
"#;
        let s: Switches = state::store::parse(text).unwrap();
        assert_eq!(s.limits.switches, 4);
        assert_eq!(s.limits.adapters_per_partition, 8);
        let prod = s.get("PRODNET").unwrap();
        assert_eq!(prod.bridge(), "qvbr1");
        assert_eq!(prod.bond().as_deref(), Some("qvbo1"));
        assert_eq!(prod.uplink.as_ref().unwrap().bond, Some(Bond::ActiveBackup));
        assert!(s.get("BACKEND").unwrap().uplink.is_none());
        assert_eq!(s.get("BACKEND").unwrap().bond(), None);
        assert_eq!(s.uplink_of("P0000-B01-D00-F0-E1").unwrap().name, "PRODNET");
        assert_eq!(s.free_id(), 3);
        let back: Switches = state::store::parse(&state::store::render(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // A key the file does not know is refused.
        assert!(state::store::parse::<Switches>("schema_version = 1\nspeed = 1\n").is_err());
    }

    #[test]
    fn kernel_names_read_back_to_what_they_name() {
        assert_eq!(tap_of(&tap_name(12, 3)), Some((12, 3)));
        assert_eq!(tap_name(999, 255).len(), 10);
        assert_eq!(tap_of("eth0"), None);
        assert_eq!(bridge_of(&bridge_name(7)), Some(7));
        assert_eq!(bond_of(&bond_name(7)), Some(7));
        assert_eq!(bridge_of("qvbo7"), None);
    }
}
