// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The system serial number: DMI's when it is a real one, otherwise one
//! derived once from the first NIC's MAC and kept in `host.toml`.

use std::path::Path;

use crate::smbios;
use crate::sysfs::text;

/// The source of a serial number DMI gave.
pub(crate) const FROM_DMI: &str = "dmi";
/// The source of a serial number derived from a MAC, kept or new.
pub(crate) const DERIVED: &str = "derived";
/// The source when there is no serial number.
pub(crate) const NONE: &str = "none";

/// What firmware writes where a serial number was never set, lower case.
const PLACEHOLDERS: &[&str] = &[
    "not specified",
    "not applicable",
    "not available",
    "to be filled by o.e.m.",
    "to be filled by oem",
    "default string",
    "system serial number",
    "none",
    "n/a",
    "na",
    "unknown",
    "invalid",
    "empty",
    "oem",
    "123456789",
    "1234567890",
];

/// `serial` trimmed, when it is a real one: not empty, not a
/// placeholder, not all zeros or all `F`.
pub(crate) fn real(serial: &str) -> Option<String> {
    let serial = serial.trim();
    let lower = serial.to_ascii_lowercase();
    let all = |c: char| lower.chars().all(|x| x == c);
    if serial.is_empty() || PLACEHOLDERS.contains(&lower.as_str()) || all('0') || all('f') {
        return None;
    }
    Some(serial.to_string())
}

/// The serial derived from a MAC: `M` and the twelve hex digits.
pub(crate) fn derived(mac: [u8; 6]) -> String {
    let digits = u64::from_be_bytes([0, 0, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]]);
    format!("M{digits:012X}")
}

/// DMI's serial when it is a real one. `product_serial` is read first;
/// when it is unreadable the type 1 structure of `table` stands in.
pub(crate) fn dmi(sys: &Path, table: &[u8]) -> Option<String> {
    match text(sys.join("class/dmi/id/product_serial")) {
        Some(serial) => real(&serial),
        None => real(&smbios::system(table)?.serial),
    }
}

/// The serial number and its source: DMI's, else the kept one, else one
/// derived from the first NIC's MAC, else none.
pub(crate) fn choose(
    dmi: Option<String>,
    kept: Option<&str>,
    first_mac: impl FnOnce() -> Option<[u8; 6]>,
) -> (String, &'static str) {
    if let Some(serial) = dmi {
        return (serial, FROM_DMI);
    }
    if let Some(kept) = kept.map(str::trim).filter(|k| !k.is_empty()) {
        return (kept.to_string(), DERIVED);
    }
    match first_mac() {
        Some(mac) => (derived(mac), DERIVED),
        None => (String::new(), NONE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Tree, structure};

    #[test]
    fn a_real_serial_is_kept_and_a_placeholder_is_not() {
        assert_eq!(real(" SN12345 \n").as_deref(), Some("SN12345"));
        assert_eq!(real("0F").as_deref(), Some("0F"));
        assert_eq!(real("VMware-56 4d").as_deref(), Some("VMware-56 4d"));
        for placeholder in [
            "",
            "   ",
            "Not Specified",
            "To Be Filled By O.E.M.",
            "to be filled by o.e.m.",
            "Default string",
            "None",
            "0",
            "System Serial Number",
            "00000000",
            "FFFFFFFF",
            "ffffffffffff",
            "N/A",
        ] {
            assert_eq!(real(placeholder), None, "{placeholder:?}");
        }
    }

    #[test]
    fn a_derived_serial_is_m_and_the_mac_in_upper_case() {
        assert_eq!(
            derived([0x52, 0x54, 0x00, 0xab, 0xcd, 0x0f]),
            "M525400ABCD0F"
        );
        assert_eq!(derived([0; 6]), "M000000000000");
    }

    #[test]
    fn the_serial_is_dmis_then_the_kept_one_then_a_derived_one() {
        let mac = || Some([0x52, 0x54, 0, 0x12, 0x34, 0x56]);
        let no_mac = || None;
        let is = |serial: &str, source: &'static str| (serial.to_string(), source);
        assert_eq!(
            choose(Some("SN1".into()), Some("KEPT"), mac),
            is("SN1", "dmi")
        );
        assert_eq!(choose(None, Some("KEPT"), mac), is("KEPT", "derived"));
        assert_eq!(
            choose(None, Some("  "), mac),
            is("M525400123456", "derived")
        );
        assert_eq!(choose(None, None, mac), is("M525400123456", "derived"));
        assert_eq!(choose(None, None, no_mac), is("", "none"));
        assert_eq!(choose(None, Some("KEPT"), no_mac), is("KEPT", "derived"));
    }

    #[test]
    fn the_dmi_file_is_read_before_the_raw_table() {
        let table = structure(1, &[0, 0, 0, 1], &["TABLE77"]);
        let tree = Tree::new("serial-dmi");
        let sys = tree.sys();
        assert_eq!(dmi(&sys, &table).as_deref(), Some("TABLE77"));
        assert_eq!(dmi(&sys, &[]), None);
        tree.file("sys/class/dmi/id/product_serial", "FILE42\n");
        assert_eq!(dmi(&sys, &table).as_deref(), Some("FILE42"));
        // A readable placeholder is the firmware's answer.
        tree.file("sys/class/dmi/id/product_serial", "Not Specified\n");
        assert_eq!(dmi(&sys, &table), None);
    }

    #[test]
    fn a_placeholder_in_the_raw_table_is_no_serial() {
        let table = structure(1, &[0, 0, 0, 1], &["Default string"]);
        let tree = Tree::new("serial-table");
        assert_eq!(dmi(&tree.sys(), &table), None);
    }
}
