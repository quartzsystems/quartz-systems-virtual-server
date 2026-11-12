// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The Ethernet links of `/sys/class/net`: the ones backed by a device,
//! in interface index order. The first is "the first NIC" a default
//! host name and a derived serial number take their MAC from.

use std::fs;
use std::path::Path;

/// `ARPHRD_ETHER`.
const ETHERNET: &str = "1";

/// One Ethernet link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Interface name.
    pub name: String,
    /// Interface index.
    pub index: u32,
    /// Hardware address.
    pub mac: [u8; 6],
}

/// `52:54:00:12:34:56`, either case.
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let parts: Vec<u8> = text
        .trim()
        .split(':')
        .map(|p| {
            (p.len() == 2)
                .then(|| u8::from_str_radix(p, 16).ok())
                .flatten()
        })
        .collect::<Option<_>>()?;
    parts.try_into().ok()
}

/// `52:54:00:12:34:56`.
pub fn format_mac(mac: [u8; 6]) -> String {
    let parts: Vec<String> = mac.iter().map(|b| format!("{b:02x}")).collect();
    parts.join(":")
}

/// The last six hex digits of a MAC, upper case.
pub fn mac_tail(mac: [u8; 6]) -> String {
    format!("{:02X}{:02X}{:02X}", mac[3], mac[4], mac[5])
}

/// The Ethernet links under `sys` (`/sys`) that a device backs, by index.
pub fn ethernet_in(sys: &Path) -> Vec<Link> {
    let read = |dir: &Path, file: &str| {
        fs::read_to_string(dir.join(file))
            .map(|t| t.trim().to_string())
            .ok()
    };
    let mut links: Vec<Link> = fs::read_dir(sys.join("class/net"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            // A sub-interface or a bridge has no device behind it.
            if !dir.join("device").exists() || read(&dir, "type")? != ETHERNET {
                return None;
            }
            Some(Link {
                name: entry.file_name().to_string_lossy().into_owned(),
                index: read(&dir, "ifindex")?.parse().ok()?,
                mac: parse_mac(&read(&dir, "address")?)?,
            })
        })
        .collect();
    links.sort_by_key(|l| l.index);
    links
}

/// The Ethernet links of this host.
pub fn ethernet() -> Vec<Link> {
    ethernet_in(Path::new("/sys"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macs_parse_and_print() {
        let mac = parse_mac("52:54:00:AB:cd:0f\n").unwrap();
        assert_eq!(mac, [0x52, 0x54, 0, 0xab, 0xcd, 0x0f]);
        assert_eq!(format_mac(mac), "52:54:00:ab:cd:0f");
        assert_eq!(mac_tail(mac), "ABCD0F");
        for bad in [
            "",
            "52:54:00:ab:cd",
            "52:54:00:ab:cd:0f:11",
            "5:54:00:ab:cd:0f",
            "eth0",
        ] {
            assert_eq!(parse_mac(bad), None, "{bad}");
        }
    }

    #[test]
    fn links_are_the_device_backed_ethernet_ones_by_index() {
        let sys = std::env::temp_dir().join(format!("appliance-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&sys);
        let link = |name: &str, index: u32, ty: &str, mac: &str, device: bool| {
            let dir = sys.join("class/net").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("ifindex"), format!("{index}\n")).unwrap();
            fs::write(dir.join("type"), format!("{ty}\n")).unwrap();
            fs::write(dir.join("address"), format!("{mac}\n")).unwrap();
            if device {
                fs::create_dir_all(dir.join("device")).unwrap();
            }
        };
        link("lo", 1, "772", "00:00:00:00:00:00", false);
        link("eth1", 3, "1", "52:54:00:00:00:02", true);
        link("eth0", 2, "1", "52:54:00:00:00:01", true);
        link("eth0.100", 4, "1", "52:54:00:00:00:01", false);
        let found = ethernet_in(&sys);
        let names: Vec<&str> = found.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["eth0", "eth1"]);
        assert_eq!(found[0].mac[5], 1);
        fs::remove_dir_all(&sys).unwrap();
    }
}
