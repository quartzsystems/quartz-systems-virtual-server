// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The system as the firmware names it, and its serial number.

pub(crate) mod serial;

use state::doc::inventory::System;

use crate::sysfs::text;
use crate::{Roots, smbios};

/// The system. `table` is the raw SMBIOS table, `kept_serial` the serial
/// number `host.toml` keeps.
pub(crate) fn read(roots: &Roots, table: &[u8], kept_serial: Option<&str>) -> System {
    let record = smbios::system(table).unwrap_or_default();
    let id = roots.sys.join("class/dmi/id");
    let named = |file: &str, fallback: String| {
        text(id.join(file))
            .filter(|t| !t.is_empty())
            .unwrap_or(fallback)
    };
    let first_mac = || {
        appliance::links::ethernet_in(&roots.sys)
            .first()
            .map(|link| link.mac)
    };
    let (serial, source) = serial::choose(serial::dmi(&roots.sys, table), kept_serial, first_mac);
    System {
        manufacturer: named("sys_vendor", record.manufacturer),
        product: named("product_name", record.product),
        serial,
        serial_source: source.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Tree, structure};

    fn nic(tree: &Tree, name: &str, index: u32, mac: &str) {
        let dir = format!("sys/class/net/{name}");
        tree.file(&format!("{dir}/ifindex"), format!("{index}\n"));
        tree.file(&format!("{dir}/type"), "1\n");
        tree.file(&format!("{dir}/address"), format!("{mac}\n"));
        tree.dir(&format!("{dir}/device"));
    }

    #[test]
    fn the_names_come_from_dmi_and_a_real_serial_wins() {
        let tree = Tree::new("system-dmi");
        tree.file("sys/class/dmi/id/sys_vendor", "Quartz Systems\n");
        tree.file("sys/class/dmi/id/product_name", "Model 9\n");
        tree.file("sys/class/dmi/id/product_serial", "SN12345\n");
        nic(&tree, "eth0", 2, "52:54:00:12:34:56");
        let system = read(&tree.roots(), &[], Some("KEPT01"));
        assert_eq!(system.manufacturer, "Quartz Systems");
        assert_eq!(system.product, "Model 9");
        assert_eq!(system.serial, "SN12345");
        assert_eq!(system.serial_source, "dmi");
    }

    #[test]
    fn a_placeholder_serial_gives_way_to_the_kept_one() {
        let tree = Tree::new("system-kept");
        tree.file(
            "sys/class/dmi/id/product_serial",
            "To Be Filled By O.E.M.\n",
        );
        nic(&tree, "eth0", 2, "52:54:00:12:34:56");
        let system = read(&tree.roots(), &[], Some("M0A0B0C0D0E0F"));
        assert_eq!(system.serial, "M0A0B0C0D0E0F");
        assert_eq!(system.serial_source, "derived");
    }

    #[test]
    fn with_nothing_kept_the_serial_is_derived_from_the_first_nic() {
        let tree = Tree::new("system-derived");
        tree.file("sys/class/dmi/id/product_serial", "Not Specified\n");
        nic(&tree, "eth1", 3, "52:54:00:00:00:02");
        nic(&tree, "eth0", 2, "52:54:00:ab:cd:ef");
        let system = read(&tree.roots(), &[], None);
        assert_eq!(system.serial, "M525400ABCDEF");
        assert_eq!(system.serial_source, "derived");
    }

    #[test]
    fn with_no_dmi_serial_and_no_nic_there_is_no_serial() {
        let tree = Tree::new("system-none");
        let system = read(&tree.roots(), &[], None);
        assert_eq!(system.serial, "");
        assert_eq!(system.serial_source, "none");
    }

    #[test]
    fn the_raw_table_stands_in_when_the_dmi_files_are_unreadable() {
        let tree = Tree::new("system-table");
        let table = structure(1, &[1, 2, 0, 3], &["Quartz", "Model 9", "TABLE77"]);
        let system = read(&tree.roots(), &table, None);
        assert_eq!(system.manufacturer, "Quartz");
        assert_eq!(system.product, "Model 9");
        assert_eq!(system.serial, "TABLE77");
        assert_eq!(system.serial_source, "dmi");
    }
}
