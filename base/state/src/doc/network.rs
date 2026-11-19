// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `network.toml`: the appliance's network interfaces, each with the
//! role the appliance names. IPv4 only; IPv6 is link-local and needs no
//! configuration. Version 1 described one interface; it is version 2's
//! first, with the appliance's first role.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use appliance::links::{format_mac, parse_mac};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::store::{Document, Migration};

/// Which device carries an interface.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub enum Device {
    /// The first Ethernet link with carrier.
    #[default]
    Auto,
    /// The link with this name.
    Name(String),
    /// The link with this hardware address.
    Mac([u8; 6]),
}

/// How [`Device::Auto`] is written.
const AUTO: &str = "auto";

impl TryFrom<String> for Device {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        if s == AUTO {
            return Ok(Device::Auto);
        }
        if let Some(mac) = parse_mac(&s) {
            return Ok(Device::Mac(mac));
        }
        // A kernel interface name: 15 bytes, no slash, no white space.
        let ok = !s.is_empty()
            && s.len() <= 15
            && s.bytes()
                .all(|b| b.is_ascii_graphic() && b != b'/' && b != b':');
        if ok {
            Ok(Device::Name(s))
        } else {
            Err(format!("`{s}` is not a device (a name, a MAC or `{AUTO}`)"))
        }
    }
}

impl From<Device> for String {
    fn from(d: Device) -> Self {
        d.to_string()
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Device::Auto => f.write_str(AUTO),
            Device::Name(n) => f.write_str(n),
            Device::Mac(m) => f.write_str(&format_mac(*m)),
        }
    }
}

/// An IPv4 address with its prefix length, `192.0.2.10/24`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub struct Cidr {
    /// The address.
    pub addr: Ipv4Addr,
    /// The prefix length, 1 to 32.
    pub prefix: u8,
}

impl FromStr for Cidr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || format!("`{s}` is not an address with a prefix (192.0.2.10/24)");
        let (addr, prefix) = s.split_once('/').ok_or_else(bad)?;
        let addr: Ipv4Addr = addr.parse().map_err(|_| bad())?;
        let prefix: u8 = prefix.parse().map_err(|_| bad())?;
        if !(1..=32).contains(&prefix) || addr.is_unspecified() {
            return Err(bad());
        }
        Ok(Cidr { addr, prefix })
    }
}

impl TryFrom<String> for Cidr {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Cidr> for String {
    fn from(c: Cidr) -> Self {
        c.to_string()
    }
}

impl fmt::Display for Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix)
    }
}

impl Cidr {
    /// Whether `other` is on this address's subnet.
    pub fn contains(&self, other: Ipv4Addr) -> bool {
        let mask = u32::MAX
            .checked_shl(32 - u32::from(self.prefix))
            .unwrap_or(0);
        u32::from(self.addr) & mask == u32::from(other) & mask
    }
}

/// Where the address comes from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// From a DHCP server.
    #[default]
    Dhcp,
    /// From this file.
    Static,
}

fn yes() -> bool {
    true
}

// By reference, as serde's `skip_serializing_if` calls it.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(b: &bool) -> bool {
    *b
}

/// One network interface.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// What the appliance uses it for, a role it names; empty for its
    /// first role.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    /// The device: a name, a MAC, or `auto`.
    #[serde(default)]
    pub device: Device,
    /// `dhcp` or `static`.
    #[serde(default)]
    pub mode: Mode,
    /// The static address and prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Cidr>,
    /// The static default gateway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<Ipv4Addr>,
    /// The 802.1Q VLAN the interface is on, 1 to 4094.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    /// Name servers; with DHCP they come before the lease's.
    #[serde(default)]
    pub dns: Vec<IpAddr>,
    /// The default route is through it: its static gateway, or its
    /// lease's router. One interface at most.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub carries_route: bool,
    /// Its name servers, and its lease's, are the resolver's.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub carries_dns: bool,
    /// The time servers are reached through it: the clock is kept once
    /// it has an address.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub carries_time: bool,
}

impl Default for Interface {
    /// DHCP on the first Ethernet link with carrier, carrying the
    /// route, the name servers and the time.
    fn default() -> Self {
        Self {
            role: String::new(),
            device: Device::Auto,
            mode: Mode::Dhcp,
            address: None,
            gateway: None,
            vlan: None,
            dns: Vec::new(),
            carries_route: true,
            carries_dns: true,
            carries_time: true,
        }
    }
}

impl Interface {
    /// Whether the values make sense together.
    pub fn validate(&self) -> Result<(), String> {
        match (self.mode, self.address, self.gateway) {
            (Mode::Static, None, _) => return Err("a static interface needs an address".into()),
            (Mode::Static, Some(a), Some(g)) if !a.contains(g) || g == a.addr => {
                return Err(format!("the gateway {g} is not on the subnet of {a}"));
            }
            (Mode::Dhcp, Some(_), _) | (Mode::Dhcp, _, Some(_)) => {
                return Err("a DHCP interface takes no address or gateway".into());
            }
            _ => {}
        }
        if self.gateway.is_some() && !self.carries_route {
            return Err("a gateway is for the interface that carries the default route".into());
        }
        if let Some(vlan) = self.vlan
            && !(1..=4094).contains(&vlan)
        {
            return Err(format!("{vlan} is not a VLAN ID (1 to 4094)"));
        }
        Ok(())
    }
}

fn one() -> Vec<Interface> {
    vec![Interface::default()]
}

/// `network.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Network {
    /// The interfaces, the appliance's first role first. A file with
    /// none has the default one.
    #[serde(default = "one", rename = "interface")]
    pub interfaces: Vec<Interface>,
}

impl Default for Network {
    /// One interface, [`Interface::default`].
    fn default() -> Self {
        Self { interfaces: one() }
    }
}

impl From<Interface> for Network {
    fn from(interface: Interface) -> Self {
        Self {
            interfaces: vec![interface],
        }
    }
}

impl Network {
    /// The first interface: the appliance's first role, its management
    /// network.
    pub fn primary(&self) -> &Interface {
        &self.interfaces[0]
    }

    /// This network with `interface` in place of the first, its role
    /// kept; the others as they are.
    #[must_use]
    pub fn with_primary(mut self, mut interface: Interface) -> Self {
        interface.role = std::mem::take(&mut self.interfaces[0].role);
        self.interfaces[0] = interface;
        self
    }
}

/// Version 1's table, one interface, as version 2's: that interface.
pub fn from_version_1(table: &mut Table) -> Result<(), String> {
    let interface = std::mem::take(table);
    table.insert(
        "interface".into(),
        Value::Array(vec![Value::Table(interface)]),
    );
    Ok(())
}

impl Document for Network {
    const FILE: &'static str = "network.toml";
    const VERSION: u32 = 2;
    const MIGRATIONS: &'static [Migration] = &[from_version_1];

    fn validate(&self) -> Result<(), String> {
        if self.interfaces.is_empty() {
            return Err("there is no interface".into());
        }
        for (i, interface) in self.interfaces.iter().enumerate() {
            let which = if interface.role.is_empty() {
                format!("interface {}", i + 1)
            } else {
                format!("interface {} ({})", i + 1, interface.role)
            };
            interface.validate().map_err(|e| {
                if self.interfaces.len() == 1 {
                    e
                } else {
                    format!("{which}: {e}")
                }
            })?;
            let before = &self.interfaces[..i];
            if before.iter().any(|o| o.role == interface.role) {
                return Err(format!("{which}: its role is another's"));
            }
            let same_place = |o: &&Interface| {
                o.device == interface.device && o.vlan == interface.vlan && o.device != Device::Auto
            };
            if before.iter().any(|o| same_place(&o)) {
                return Err(format!("{which}: its device and VLAN are another's"));
            }
        }
        if self.interfaces.iter().filter(|i| i.carries_route).count() > 1 {
            return Err("only one interface carries the default route".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{parse, render};

    #[test]
    fn an_empty_file_is_dhcp_on_the_first_link_with_carrier() {
        for text in ["schema_version = 1\n", "schema_version = 2\n"] {
            let net: Network = parse(text).unwrap();
            assert_eq!(net, Network::default());
            let first = net.primary();
            assert_eq!((&first.device, first.mode), (&Device::Auto, Mode::Dhcp));
            assert!(first.carries_route && first.carries_dns && first.carries_time);
        }
    }

    #[test]
    fn a_version_1_file_is_the_same_interface_in_version_2() {
        let v1 = "schema_version = 1\ndevice = \"eth1\"\nmode = \"static\"\n\
                  address = \"192.0.2.10/24\"\ngateway = \"192.0.2.1\"\nvlan = 100\n\
                  dns = [\"192.0.2.53\"]\n";
        let net: Network = parse(v1).unwrap();
        assert_eq!(net.interfaces.len(), 1);
        let i = net.primary();
        assert_eq!(i.role, "");
        assert_eq!(i.device, Device::Name("eth1".into()));
        assert_eq!(i.address.unwrap().to_string(), "192.0.2.10/24");
        assert_eq!((i.vlan, i.dns.len()), (Some(100), 1));
        assert!(i.carries_route && i.carries_dns && i.carries_time);
        // Written again, it is version 2 with nothing added but the table.
        let text = render(&net).unwrap();
        assert!(text.contains("schema_version = 2"), "{text}");
        assert!(text.contains("[[interface]]"), "{text}");
        assert!(
            !text.contains("role") && !text.contains("carries"),
            "{text}"
        );
        assert_eq!(parse::<Network>(&text).unwrap(), net);
    }

    #[test]
    fn two_interfaces_with_their_roles_round_trip() {
        let text = "schema_version = 2\n\
                    [[interface]]\nrole = \"public\"\nmode = \"static\"\n\
                    address = \"192.0.2.10/24\"\ngateway = \"192.0.2.1\"\n\
                    [[interface]]\nrole = \"private\"\ndevice = \"eth1\"\nvlan = 20\n\
                    carries_route = false\ncarries_dns = false\ncarries_time = false\n";
        let net: Network = parse(text).unwrap();
        assert_eq!(net.interfaces.len(), 2);
        assert_eq!(net.primary().role, "public");
        let private = &net.interfaces[1];
        assert_eq!((private.vlan, private.carries_route), (Some(20), false));
        assert_eq!(parse::<Network>(&render(&net).unwrap()).unwrap(), net);
        // The console's change of the first leaves the second and the role.
        let changed = net.clone().with_primary(Interface::default());
        assert_eq!(changed.primary().role, "public");
        assert_eq!(changed.primary().mode, Mode::Dhcp);
        assert_eq!(changed.interfaces[1], net.interfaces[1]);
    }

    #[test]
    fn interfaces_that_clash_are_refused() {
        let head = "schema_version = 2\n[[interface]]\nrole = \"a\"\n[[interface]]\n";
        for (rest, why) in [
            ("role = \"a\"\ncarries_route = false\n", "role"),
            ("role = \"b\"\n", "default route"),
            (
                "role = \"b\"\ncarries_route = false\nmode = \"static\"\naddress = \"10.0.0.2/24\"\ngateway = \"10.0.0.1\"\n",
                "gateway",
            ),
        ] {
            let e = parse::<Network>(&format!("{head}{rest}"))
                .unwrap_err()
                .to_string();
            assert!(e.contains(why), "{rest}: {e}");
        }
        let same = "schema_version = 2\n[[interface]]\ndevice = \"eth0\"\n\
                    [[interface]]\nrole = \"b\"\ndevice = \"eth0\"\ncarries_route = false\n";
        assert!(
            parse::<Network>(same)
                .unwrap_err()
                .to_string()
                .contains("device")
        );
        let vlans = "schema_version = 2\n[[interface]]\ndevice = \"eth0\"\n\
                     [[interface]]\nrole = \"b\"\ndevice = \"eth0\"\nvlan = 20\ncarries_route = false\n";
        assert!(parse::<Network>(vlans).is_ok());
        assert!(parse::<Network>("schema_version = 2\ninterface = []\n").is_err());
    }

    #[test]
    fn a_static_vlan_interface_round_trips() {
        let text = "schema_version = 1\ndevice = \"52:54:00:12:34:56\"\nmode = \"static\"\n\
                    address = \"192.0.2.10/24\"\ngateway = \"192.0.2.1\"\nvlan = 100\n\
                    dns = [\"192.0.2.53\", \"2001:db8::53\"]\n";
        let net: Network = parse(text).unwrap();
        let first = net.primary();
        assert_eq!(first.device, Device::Mac([0x52, 0x54, 0, 0x12, 0x34, 0x56]));
        assert_eq!(first.address.unwrap().to_string(), "192.0.2.10/24");
        assert_eq!(first.vlan, Some(100));
        assert_eq!(first.dns.len(), 2);
        assert_eq!(parse::<Network>(&render(&net).unwrap()).unwrap(), net);
        let named: Network = parse("schema_version = 1\ndevice = \"eth1\"\n").unwrap();
        assert_eq!(named.primary().device, Device::Name("eth1".into()));
    }

    #[test]
    fn bad_values_are_refused() {
        for text in [
            "mode = \"static\"",
            "mode = \"bootp\"",
            "address = \"192.0.2.10/24\"",
            "gateway = \"192.0.2.1\"",
            "mode = \"static\"\naddress = \"192.0.2.10\"",
            "mode = \"static\"\naddress = \"192.0.2.10/0\"",
            "mode = \"static\"\naddress = \"192.0.2.10/33\"",
            "mode = \"static\"\naddress = \"192.0.2.10/24\"\ngateway = \"198.51.100.1\"",
            "mode = \"static\"\naddress = \"192.0.2.10/24\"\ngateway = \"192.0.2.10\"",
            "vlan = 0",
            "vlan = 4095",
            "device = \"\"",
            "device = \"a/b\"",
            "device = \"an-interface-name-too-long\"",
            "dns = [\"nowhere\"]",
            "prefix = 24",
        ] {
            let text = format!("schema_version = 1\n{text}\n");
            assert!(parse::<Network>(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_subnet_holds_its_addresses() {
        let c: Cidr = "10.1.2.3/23".parse().unwrap();
        assert!(c.contains(Ipv4Addr::new(10, 1, 3, 254)));
        assert!(!c.contains(Ipv4Addr::new(10, 1, 4, 1)));
        let host: Cidr = "10.1.2.3/32".parse().unwrap();
        assert!(host.contains(Ipv4Addr::new(10, 1, 2, 3)));
    }
}
