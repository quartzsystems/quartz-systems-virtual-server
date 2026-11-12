// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `network.toml`: the management interface. IPv4 only; IPv6 is
//! link-local and needs no configuration.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;

use appliance::links::{format_mac, parse_mac};
use serde::{Deserialize, Serialize};

use crate::store::{Document, Migration};

/// Which device carries the management interface.
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

/// `network.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Network {
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
}

impl Document for Network {
    const FILE: &'static str = "network.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
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
        if let Some(vlan) = self.vlan
            && !(1..=4094).contains(&vlan)
        {
            return Err(format!("{vlan} is not a VLAN ID (1 to 4094)"));
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
        let net: Network = parse("schema_version = 1\n").unwrap();
        assert_eq!(net, Network::default());
        assert_eq!((net.device, net.mode), (Device::Auto, Mode::Dhcp));
    }

    #[test]
    fn a_static_vlan_interface_round_trips() {
        let text = "schema_version = 1\ndevice = \"52:54:00:12:34:56\"\nmode = \"static\"\n\
                    address = \"192.0.2.10/24\"\ngateway = \"192.0.2.1\"\nvlan = 100\n\
                    dns = [\"192.0.2.53\", \"2001:db8::53\"]\n";
        let net: Network = parse(text).unwrap();
        assert_eq!(net.device, Device::Mac([0x52, 0x54, 0, 0x12, 0x34, 0x56]));
        assert_eq!(net.address.unwrap().to_string(), "192.0.2.10/24");
        assert_eq!(net.vlan, Some(100));
        assert_eq!(net.dns.len(), 2);
        assert_eq!(parse::<Network>(&render(&net).unwrap()).unwrap(), net);
        let named: Network = parse("schema_version = 1\ndevice = \"eth1\"\n").unwrap();
        assert_eq!(named.device, Device::Name("eth1".into()));
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
