// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `host.toml`: the host's name, its display time zone and its time
//! servers. The file is there once the host has been configured.

use appliance::links::mac_tail;
use appliance::time::Zone;
use serde::{Deserialize, Serialize};

use crate::store::{Document, Migration};

fn utc() -> String {
    Zone::UTC.to_string()
}

/// `host.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Host {
    /// The host name; absent, the default from the first NIC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The display zone, `UTC` or `UTC+hh:mm`. The system clock is UTC.
    #[serde(default = "utc")]
    pub time_zone: String,
    /// Time servers, `host` or `host:port`, tried in order.
    #[serde(default)]
    pub ntp_servers: Vec<String>,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            name: None,
            time_zone: utc(),
            ntp_servers: Vec::new(),
        }
    }
}

/// Whether `name` is a host name: one label of letters, digits and
/// hyphens, 63 characters at most, no hyphen first or last.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// A time server as written: the host and the port (123 when absent).
pub fn split_server(server: &str) -> Option<(&str, u16)> {
    let (host, port) = match server.rsplit_once(':') {
        // A bare IPv6 address has colons and no port.
        Some((h, p)) if !h.contains(':') => (h, p.parse().ok()?),
        _ => (server, 123),
    };
    let ok = !host.is_empty() && !host.contains(|c: char| c.is_whitespace() || c == '/');
    ok.then_some((host, port))
}

impl Host {
    /// The default name: `prefix`, then the last six hex digits of the
    /// first NIC's MAC (zeros without one).
    pub fn default_name(prefix: &str, first_mac: Option<[u8; 6]>) -> String {
        format!("{prefix}{}", mac_tail(first_mac.unwrap_or_default()))
    }

    /// The name in use.
    pub fn name_or_default(&self, prefix: &str, first_mac: Option<[u8; 6]>) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| Self::default_name(prefix, first_mac))
    }

    /// The display zone.
    pub fn zone(&self) -> Zone {
        self.time_zone.parse().unwrap_or_default()
    }
}

impl Document for Host {
    const FILE: &'static str = "host.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        if let Some(name) = &self.name
            && !valid_name(name)
        {
            return Err(format!("`{name}` is not a host name"));
        }
        self.time_zone.parse::<Zone>()?;
        for server in &self.ntp_servers {
            if split_server(server).is_none() {
                return Err(format!("`{server}` is not a time server"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{parse, render};

    #[test]
    fn an_empty_file_is_the_defaults() {
        let host: Host = parse("schema_version = 1\n").unwrap();
        assert_eq!(host, Host::default());
        assert_eq!(host.zone(), Zone::UTC);
        let mac = [0x52, 0x54, 0, 0x12, 0xab, 0xcd];
        assert_eq!(host.name_or_default("QSDEMO", Some(mac)), "QSDEMO12ABCD");
        assert_eq!(Host::default_name("QSDEMO", None), "QSDEMO000000");
    }

    #[test]
    fn it_round_trips() {
        let host = Host {
            name: Some("vs-01".into()),
            time_zone: "UTC+01:00".into(),
            ntp_servers: vec!["time.example".into(), "10.0.2.2:12300".into()],
        };
        assert_eq!(parse::<Host>(&render(&host).unwrap()).unwrap(), host);
    }

    #[test]
    fn bad_values_are_refused() {
        for text in [
            "name = \"-x\"",
            "name = \"a b\"",
            "name = \"\"",
            "time_zone = \"Europe/Berlin\"",
            "ntp_servers = [\"\"]",
            "ntp_servers = [\"a:b\"]",
            "ntp_servers = [\"a b\"]",
            "serial = \"M525400123456\"",
            "host_name = \"x\"",
        ] {
            let text = format!("schema_version = 1\n{text}\n");
            assert!(parse::<Host>(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn servers_split_into_host_and_port() {
        assert_eq!(split_server("time.example"), Some(("time.example", 123)));
        assert_eq!(split_server("10.0.2.2:12300"), Some(("10.0.2.2", 12300)));
        assert_eq!(split_server("fe80::1"), Some(("fe80::1", 123)));
        assert_eq!(split_server(":123"), None);
        assert_eq!(split_server("a:99999"), None);
    }
}
