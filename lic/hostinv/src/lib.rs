// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host inventory, read at every IPL from sysfs, procfs and DMI:
//! processors, memory, disks, network interfaces and PCI devices, each
//! device with a location code (`docs/LOCATION.md`).
//!
//! - [`read`]: the whole inventory, for `inventory.toml`.
//! - [`dmi_serial`], [`derived_serial`]: the system serial number.
//! - [`oem_strings`]: the SMBIOS OEM strings.
//! - [`location`]: the location code format.
//! - [`smbios`]: the SMBIOS structure table.
//!
//! Nothing here fails: a node that is missing reads as empty or zero.

#![forbid(unsafe_code)]

pub mod location;
pub mod smbios;

mod read;
mod sysfs;
mod system;

#[cfg(test)]
mod fixture;

use std::path::PathBuf;

/// Where the kernel's file systems are; `Roots::default()` is `/sys`
/// and `/proc`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roots {
    /// sysfs.
    pub sys: PathBuf,
    /// procfs.
    pub proc: PathBuf,
}

impl Default for Roots {
    fn default() -> Self {
        Roots {
            sys: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
        }
    }
}

/// Read the whole inventory. `kept_serial` is the serial number already
/// kept in `host.toml`, if any.
///
/// The serial number is DMI's when DMI has a real one (source `dmi`);
/// otherwise the kept one, or the one derived from the first NIC's MAC
/// (source `derived`); with neither it is empty and the source `none`.
pub fn read(roots: &Roots, kept_serial: Option<&str>) -> state::Inventory {
    read::inventory(roots, kept_serial)
}

/// The serial DMI gives, when it is a real one: not empty and not a
/// placeholder such as `Not Specified` or `To Be Filled By O.E.M.`.
pub fn dmi_serial(roots: &Roots) -> Option<String> {
    system::serial::dmi(&roots.sys, &smbios::table(&roots.sys))
}

/// The serial derived from a MAC: `M` and the twelve hex digits, upper
/// case.
pub fn derived_serial(mac: [u8; 6]) -> String {
    system::serial::derived(mac)
}

/// The SMBIOS OEM strings (type 11), in order; empty when the table is
/// unreadable.
pub fn oem_strings(roots: &Roots) -> Vec<String> {
    smbios::oem_strings(&smbios::table(&roots.sys))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Tree;

    #[test]
    fn the_default_roots_are_sys_and_proc() {
        let roots = Roots::default();
        assert_eq!(roots.sys, PathBuf::from("/sys"));
        assert_eq!(roots.proc, PathBuf::from("/proc"));
    }

    #[test]
    fn an_empty_root_reads_as_an_empty_inventory() {
        let tree = Tree::new("empty");
        let inv = read(&tree.roots(), None);
        let expected = state::Inventory {
            system: state::doc::inventory::System {
                serial_source: "none".into(),
                ..Default::default()
            },
            processors: state::doc::inventory::Processors {
                virtualization: "none".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(inv, expected);
        assert_eq!(dmi_serial(&tree.roots()), None);
        assert!(oem_strings(&tree.roots()).is_empty());
    }

    #[test]
    fn roots_that_do_not_exist_read_as_an_empty_inventory() {
        let roots = Roots {
            sys: PathBuf::from("/nonexistent/hostinv/sys"),
            proc: PathBuf::from("/nonexistent/hostinv/proc"),
        };
        let inv = read(&roots, Some("KEPT01"));
        assert_eq!(inv.system.serial, "KEPT01");
        assert_eq!(inv.system.serial_source, "derived");
        assert!(inv.disks.is_empty() && inv.interfaces.is_empty() && inv.pci.is_empty());
    }

    #[test]
    fn the_oem_strings_come_from_the_table_under_the_root() {
        let tree = Tree::new("oem");
        let table = fixture::structure(11, &[2], &["qsvs:role=host", "second"]);
        tree.file("sys/firmware/dmi/tables/DMI", &table);
        assert_eq!(oem_strings(&tree.roots()), ["qsvs:role=host", "second"]);
    }

    #[test]
    fn the_derived_serial_is_m_and_twelve_hex_digits() {
        assert_eq!(
            derived_serial([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
            "M525400123456"
        );
    }
}
