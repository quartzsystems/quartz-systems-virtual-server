// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The three forms an installer and a service console share: the host
//! name with the management network, the time, and a service tools
//! user. Each is what was typed, the screen that shows it, and the
//! check that turns it into what the state files hold or says what is
//! wrong with it.

use std::net::{IpAddr, Ipv4Addr};

use appliance::time::{Zone, civil, from_civil};
use console::Screen;
use scrkit::ui::{self, FormLine};
use state::{Cidr, Device, Document, Host, Mode, Network};

use crate::kit;

/// One Ethernet link an operator may choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkChoice {
    /// The kernel's name.
    pub name: String,
    /// The hardware address, `52:54:00:12:34:56`.
    pub mac: String,
    /// The location code.
    pub location: String,
    /// Whether the link has carrier.
    pub carrier: bool,
}

/// The host name and the management network, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkEntry {
    /// The host name.
    pub host: String,
    /// The interface: its number in the list, or nothing for the first
    /// with carrier.
    pub interface: String,
    /// `1` for DHCP, `2` for a static address.
    pub mode: String,
    /// The static address.
    pub address: String,
    /// Its prefix length.
    pub prefix: String,
    /// The default gateway.
    pub gateway: String,
    /// The VLAN ID.
    pub vlan: String,
    /// Name servers, separated by commas or spaces.
    pub dns: String,
}

impl NetworkEntry {
    /// The entry that shows `host` and `network` over `links`.
    #[must_use]
    pub fn of(host: &str, network: &Network, links: &[LinkChoice]) -> Self {
        let at = |found: Option<usize>| found.map(|i| (i + 1).to_string()).unwrap_or_default();
        let interface = match &network.device {
            Device::Auto => String::new(),
            Device::Name(name) => at(links.iter().position(|l| &l.name == name)),
            Device::Mac(mac) => {
                let mac = appliance::links::format_mac(*mac);
                at(links.iter().position(|l| l.mac.eq_ignore_ascii_case(&mac)))
            }
        };
        let dns: Vec<String> = network.dns.iter().map(ToString::to_string).collect();
        Self {
            host: host.to_string(),
            interface,
            mode: match network.mode {
                Mode::Dhcp => "1",
                Mode::Static => "2",
            }
            .into(),
            address: network
                .address
                .map(|a| a.addr.to_string())
                .unwrap_or_default(),
            prefix: network
                .address
                .map(|a| a.prefix.to_string())
                .unwrap_or_default(),
            gateway: network.gateway.map(|g| g.to_string()).unwrap_or_default(),
            vlan: network.vlan.map(|v| v.to_string()).unwrap_or_default(),
            dns: dns.join(", "),
        }
    }

    /// The entry the fields of [`NetworkEntry::screen`] hold.
    #[must_use]
    pub fn take(fields: &[String]) -> Self {
        let field = |i: usize| {
            fields
                .get(i)
                .map(|f| f.trim().to_string())
                .unwrap_or_default()
        };
        Self {
            host: field(0),
            interface: field(1),
            mode: field(2),
            address: field(3),
            prefix: field(4),
            gateway: field(5),
            vlan: field(6),
            dns: field(7),
        }
    }

    /// The form, the interfaces under it.
    #[must_use]
    pub fn screen(&self, title: &str, system: &str, links: &[LinkChoice], keys: &str) -> Screen {
        let rows = [
            FormLine::new("Host name", 40, self.host.clone()),
            FormLine::new("Interface", 2, self.interface.clone()),
            FormLine::new("Address: 1=DHCP, 2=Static", 1, self.mode.clone()),
            FormLine::new("IP address", 15, self.address.clone()),
            FormLine::new("Prefix length", 2, self.prefix.clone()),
            FormLine::new("Gateway", 15, self.gateway.clone()),
            FormLine::new("VLAN ID", 4, self.vlan.clone()),
            FormLine::new("DNS servers", 46, self.dns.clone()),
        ];
        let mut s = ui::form(title, system, &rows, keys);
        let top = ui::BODY_ROW + rows.len() + 1;
        kit::put(
            &mut s,
            top,
            "Interfaces (blank takes the first with carrier):",
        );
        let room = ui::KEYS_ROW - top - 2;
        for (i, link) in links.iter().take(room).enumerate() {
            let carrier = if link.carrier {
                "carrier"
            } else {
                "no carrier"
            };
            kit::put(
                &mut s,
                top + 1 + i,
                &format!(
                    "  {:>2}  {}  {}  {carrier}",
                    i + 1,
                    kit::fit(&link.location, 24),
                    link.mac
                ),
            );
        }
        if links.is_empty() {
            kit::put(&mut s, top + 1, "  (No Ethernet interface was found)");
        }
        s
    }

    /// The host name and the network this entry means over `links`.
    pub fn check(&self, links: &[LinkChoice]) -> Result<(String, Network), String> {
        if !state::doc::host::valid_name(&self.host) {
            return Err(
                "Host name must be letters, digits and hyphens, 63 at most, no hyphen first or last."
                    .into(),
            );
        }
        let device = if self.interface.is_empty() {
            Device::Auto
        } else {
            let link = self
                .interface
                .parse::<usize>()
                .ok()
                .and_then(|n| links.get(n.checked_sub(1)?))
                .ok_or("Interface must be a number of the list, or blank.")?;
            let mac = appliance::links::parse_mac(&link.mac)
                .ok_or("That interface has no hardware address.")?;
            Device::Mac(mac)
        };
        let mode = match self.mode.as_str() {
            "1" => Mode::Dhcp,
            "2" => Mode::Static,
            _ => return Err("Address must be 1 (DHCP) or 2 (Static).".into()),
        };
        let (address, gateway) = match mode {
            Mode::Dhcp => {
                if !(self.address.is_empty() && self.prefix.is_empty() && self.gateway.is_empty()) {
                    return Err("With DHCP, leave address, prefix length and gateway blank.".into());
                }
                (None, None)
            }
            Mode::Static => {
                let addr: Ipv4Addr = self
                    .address
                    .parse()
                    .map_err(|_| "IP address must be an IPv4 address, as 192.0.2.10.")?;
                let prefix: u8 = self
                    .prefix
                    .parse()
                    .ok()
                    .filter(|p| (1..=32).contains(p))
                    .ok_or("Prefix length must be 1 to 32.")?;
                let gateway = if self.gateway.is_empty() {
                    None
                } else {
                    Some(
                        self.gateway
                            .parse::<Ipv4Addr>()
                            .map_err(|_| "Gateway must be an IPv4 address.")?,
                    )
                };
                (Some(Cidr { addr, prefix }), gateway)
            }
        };
        let vlan = if self.vlan.is_empty() {
            None
        } else {
            Some(
                self.vlan
                    .parse::<u16>()
                    .ok()
                    .filter(|v| (1..=4094).contains(v))
                    .ok_or("VLAN ID must be 1 to 4094, or blank.")?,
            )
        };
        let mut dns = Vec::new();
        for server in self.dns.split([',', ' ']).filter(|s| !s.is_empty()) {
            dns.push(
                server
                    .parse::<IpAddr>()
                    .map_err(|_| format!("DNS server {server} is not an IP address."))?,
            );
        }
        let network = Network {
            device,
            mode,
            address,
            gateway,
            vlan,
            dns,
        };
        network.validate().map_err(|e| sentence(&e))?;
        Ok((self.host.clone(), network))
    }
}

/// A reason as a message line words it.
fn sentence(reason: &str) -> String {
    let mut chars = reason.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
    format!("{}{}.", first.unwrap_or_default(), chars.as_str())
}

/// The time settings, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimeEntry {
    /// The offset from UTC, `+01:00`; nothing for UTC.
    pub offset: String,
    /// Time servers, separated by commas or spaces.
    pub servers: String,
    /// The date to set, `2026-11-12`; nothing to leave the clock alone.
    pub date: String,
    /// The time to set, `14:05` or `14:05:30`.
    pub time: String,
}

/// What a [`TimeEntry`] means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeChoice {
    /// The display zone, as `host.toml` has it.
    pub zone: String,
    /// The time servers.
    pub servers: Vec<String>,
    /// The clock to set, in seconds since the epoch; `None` leaves it.
    pub clock: Option<i64>,
}

fn numbers<const N: usize>(text: &str, by: char) -> Option<[i64; N]> {
    let parts: Vec<i64> = text
        .split(by)
        .map(|p| p.parse::<i64>().ok().filter(|n| *n >= 0))
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

impl TimeEntry {
    /// The entry that shows `host`'s settings; the clock is left alone.
    #[must_use]
    pub fn of(host: &Host) -> Self {
        Self {
            offset: host
                .time_zone
                .strip_prefix("UTC")
                .unwrap_or_default()
                .to_string(),
            servers: host.ntp_servers.join(", "),
            date: String::new(),
            time: String::new(),
        }
    }

    /// The entry the fields of [`TimeEntry::screen`] hold.
    #[must_use]
    pub fn take(fields: &[String]) -> Self {
        let field = |i: usize| {
            fields
                .get(i)
                .map(|f| f.trim().to_string())
                .unwrap_or_default()
        };
        Self {
            offset: field(0),
            servers: field(1),
            date: field(2),
            time: field(3),
        }
    }

    /// The form, under the clock as it is at `now`.
    #[must_use]
    pub fn screen(&self, title: &str, system: &str, now: i64, keys: &str) -> Screen {
        let lines = [
            FormLine::new("UTC offset", 6, self.offset.clone()),
            FormLine::new("Time servers", 48, self.servers.clone()),
            FormLine::new("Date", 10, self.date.clone()),
            FormLine::new("Time", 8, self.time.clone()),
        ];
        let mut s = ui::form(title, system, &lines, keys);
        let zone = self.zone().unwrap_or_default();
        let top = ui::BODY_ROW + lines.len() + 1;
        for (i, text) in [
            format!("The clock now shows {}.", zone.display(now)),
            String::new(),
            "UTC offset: +hh:mm or -hh:mm; blank is UTC. It changes what is".into(),
            "displayed. The clock itself keeps UTC.".into(),
            "Time servers: names or addresses, separated by commas. With time".into(),
            "servers the clock follows them and Date and Time are not used.".into(),
            "Date as 2026-11-12 and Time as 14:05, in the offset above; blank".into(),
            "leaves the clock as it is.".into(),
        ]
        .iter()
        .enumerate()
        {
            kit::put(&mut s, top + i, text);
        }
        s
    }

    fn zone(&self) -> Result<Zone, String> {
        let text = match self.offset.as_str() {
            "" | "0" | "+0" | "+00:00" | "-00:00" => "UTC".to_string(),
            offset => format!("UTC{offset}"),
        };
        text.parse::<Zone>()
            .map_err(|_| "UTC offset must be +hh:mm or -hh:mm, or blank.".to_string())
    }

    /// What this entry means.
    pub fn check(&self) -> Result<TimeChoice, String> {
        let zone = self.zone()?;
        let servers: Vec<String> = self
            .servers
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();
        let host = Host {
            name: None,
            time_zone: zone.to_string(),
            ntp_servers: servers.clone(),
        };
        host.validate().map_err(|e| sentence(&e))?;
        let clock = match (self.date.is_empty(), self.time.is_empty()) {
            (true, true) => None,
            _ if !servers.is_empty() => {
                return Err("With time servers, leave Date and Time blank.".into());
            }
            (false, false) => Some(self.clock(zone)?),
            _ => return Err("Give both Date and Time, or neither.".into()),
        };
        Ok(TimeChoice {
            zone: zone.to_string(),
            servers,
            clock,
        })
    }

    fn clock(&self, zone: Zone) -> Result<i64, String> {
        let [year, month, day] = numbers::<3>(&self.date, '-')
            .filter(|[y, m, d]| (2026..=2099).contains(y) && (1..=12).contains(m) && *d >= 1)
            .ok_or("Date must be as 2026-11-12.")?;
        let time = numbers::<3>(&self.time, ':')
            .or_else(|| numbers::<2>(&self.time, ':').map(|[h, m]| [h, m, 0]));
        let [hour, minute, second] = time
            .filter(|[h, m, s]| *h < 24 && *m < 60 && *s < 60)
            .ok_or("Time must be as 14:05 or 14:05:30.")?;
        let local = from_civil(year, month, day, hour, minute, second);
        // A day that the month does not have comes back as another date.
        let back = civil(local);
        if i64::from(back.day) != day || i64::from(back.month) != month {
            return Err("That month has no such day.".into());
        }
        Ok(local - i64::from(zone.minutes) * 60)
    }
}

/// A service tools user, as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserEntry {
    /// The user ID.
    pub id: String,
    /// The password.
    pub password: String,
    /// The password again.
    pub again: String,
}

impl UserEntry {
    /// The entry the fields of [`UserEntry::screen`] hold.
    #[must_use]
    pub fn take(fields: &[String]) -> Self {
        let field = |i: usize| fields.get(i).cloned().unwrap_or_default();
        Self {
            id: field(0).trim().to_string(),
            password: field(1),
            again: field(2),
        }
    }

    /// The form, the rules under it. A password is never shown again:
    /// only the ID is kept in the fields.
    #[must_use]
    pub fn screen(id: &str, title: &str, system: &str, keys: &str) -> Screen {
        let lines = [
            FormLine::new("User ID", users::ID_MAX, id.to_string()),
            FormLine::hidden("Password", 32),
            FormLine::hidden("Password again", 32),
        ];
        let mut s = ui::form(title, system, &lines, keys);
        let top = ui::BODY_ROW + lines.len() + 1;
        for (i, rule) in users::RULES.iter().enumerate() {
            kit::put(&mut s, top + i, rule);
        }
        kit::put(
            &mut s,
            top + users::RULES.len() + 1,
            "What is typed in the password fields is not shown.",
        );
        s
    }

    /// The user ID as kept and the password, when both keep the rules.
    pub fn check(&self) -> Result<(String, String), String> {
        let id = users::check_id(&self.id)?;
        users::check_password(&id, &self.password)?;
        if self.password != self.again {
            return Err("The two passwords are not the same.".into());
        }
        Ok((id, self.password.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links() -> Vec<LinkChoice> {
        vec![
            LinkChoice {
                name: "eth0".into(),
                mac: "52:54:00:12:34:56".into(),
                location: "P0000-B00-D04-F0-E0".into(),
                carrier: true,
            },
            LinkChoice {
                name: "eth1".into(),
                mac: "52:54:00:12:34:57".into(),
                location: "P0000-B00-D05-F0-E0".into(),
                carrier: false,
            },
        ]
    }

    fn entry() -> NetworkEntry {
        NetworkEntry {
            host: "vs-01".into(),
            interface: "2".into(),
            mode: "2".into(),
            address: "192.0.2.10".into(),
            prefix: "24".into(),
            gateway: "192.0.2.1".into(),
            vlan: "100".into(),
            dns: "192.0.2.53, 2001:db8::53".into(),
        }
    }

    #[test]
    fn a_network_entry_becomes_the_state_file_and_back() {
        let (host, network) = entry().check(&links()).unwrap();
        assert_eq!(host, "vs-01");
        assert_eq!(network.device.to_string(), "52:54:00:12:34:57");
        assert_eq!(network.address.unwrap().to_string(), "192.0.2.10/24");
        assert_eq!((network.vlan, network.dns.len()), (Some(100), 2));
        assert_eq!(NetworkEntry::of("vs-01", &network, &links()), entry());
        // DHCP on whatever has carrier.
        let dhcp = NetworkEntry {
            host: "a".into(),
            mode: "1".into(),
            ..NetworkEntry::default()
        };
        let (_, network) = dhcp.check(&links()).unwrap();
        assert_eq!(network, Network::default());
        assert_eq!(NetworkEntry::of("a", &network, &links()), dhcp);
    }

    #[test]
    fn each_field_says_what_is_wrong_with_it() {
        let wrong = |change: &dyn Fn(&mut NetworkEntry)| {
            let mut e = entry();
            change(&mut e);
            e.check(&links()).unwrap_err()
        };
        assert!(wrong(&|e| e.host = "-x".into()).starts_with("Host name"));
        assert!(wrong(&|e| e.interface = "3".into()).starts_with("Interface"));
        assert!(wrong(&|e| e.mode = "3".into()).starts_with("Address"));
        assert!(wrong(&|e| e.address = "192.0.2".into()).starts_with("IP address"));
        assert!(wrong(&|e| e.prefix = "33".into()).starts_with("Prefix"));
        assert!(wrong(&|e| e.gateway = "x".into()).starts_with("Gateway"));
        assert!(wrong(&|e| e.gateway = "198.51.100.1".into()).contains("subnet"));
        assert!(wrong(&|e| e.vlan = "4095".into()).starts_with("VLAN"));
        assert!(wrong(&|e| e.dns = "ns1".into()).starts_with("DNS"));
        assert!(wrong(&|e| e.mode = "1".into()).starts_with("With DHCP"));
    }

    #[test]
    fn the_network_form_lists_the_interfaces_with_carrier() {
        let s = entry().screen("Network", "SYS", &links(), "F12=Cancel");
        assert_eq!(s.fields.len(), 8);
        let dump = s.dump();
        assert!(dump.contains("Host name"));
        assert!(dump.contains("1  P0000-B00-D04-F0-E0       52:54:00:12:34:56  carrier"));
        assert!(dump.contains("2  P0000-B00-D05-F0-E0       52:54:00:12:34:57  no carrier"));
        assert_eq!(NetworkEntry::take(&[" vs-01 ".into()]).host, "vs-01");
    }

    #[test]
    fn a_time_entry_is_a_zone_servers_and_perhaps_a_clock() {
        let entry = TimeEntry {
            offset: "+01:00".into(),
            servers: String::new(),
            date: "2026-11-12".into(),
            time: "14:05".into(),
        };
        let choice = entry.check().unwrap();
        assert_eq!(choice.zone, "UTC+01:00");
        // 2026-11-12 13:05:00 UTC.
        assert_eq!(choice.clock, Some(1_794_488_700));
        let utc = TimeEntry {
            servers: "time1.example, 192.0.2.123:123".into(),
            ..TimeEntry::default()
        };
        let choice = utc.check().unwrap();
        assert_eq!((choice.zone.as_str(), choice.clock), ("UTC", None));
        assert_eq!(choice.servers.len(), 2);
        let host = Host {
            name: None,
            time_zone: "UTC-05:30".into(),
            ntp_servers: vec!["a".into()],
        };
        assert_eq!(TimeEntry::of(&host).offset, "-05:30");
        assert_eq!(TimeEntry::of(&host).check().unwrap().zone, "UTC-05:30");
    }

    #[test]
    fn a_time_entry_says_what_is_wrong_with_it() {
        let wrong = |offset: &str, servers: &str, date: &str, time: &str| {
            TimeEntry {
                offset: offset.into(),
                servers: servers.into(),
                date: date.into(),
                time: time.into(),
            }
            .check()
            .unwrap_err()
        };
        assert!(wrong("Berlin", "", "", "").starts_with("UTC offset"));
        assert!(wrong("", "a/b", "", "").contains("time server"));
        assert!(wrong("", "a", "2026-11-12", "10:00").starts_with("With time servers"));
        assert!(wrong("", "", "2026-11-12", "").starts_with("Give both"));
        assert!(wrong("", "", "12.11.2026", "10:00").starts_with("Date"));
        assert!(wrong("", "", "2026-11-12", "25:00").starts_with("Time"));
        assert!(wrong("", "", "2026-02-30", "10:00").contains("no such day"));
    }

    #[test]
    fn a_user_entry_keeps_the_rules_and_wants_the_password_twice() {
        let entry =
            UserEntry::take(&[" admin ".into(), "pass word 1".into(), "pass word 1".into()]);
        assert_eq!(entry.check(), Ok(("ADMIN".into(), "pass word 1".into())));
        let other = UserEntry {
            again: "pass word 2".into(),
            ..entry.clone()
        };
        assert!(other.check().unwrap_err().contains("not the same"));
        let short = UserEntry {
            password: "x1".into(),
            again: "x1".into(),
            ..entry
        };
        assert!(short.check().unwrap_err().contains("at least 8"));
        let s = UserEntry::screen("ADMIN", "User", "SYS", "");
        assert_eq!(s.fields.len(), 3);
        assert!(s.dump().contains(users::RULES[1]));
        assert!(s.fields[1].nondisplay && s.fields[2].nondisplay);
    }
}
