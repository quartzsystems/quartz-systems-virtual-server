// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The readers, one per part of the inventory.
//!
//! - [`cpu`], [`memory`]: `/proc/cpuinfo`, `/proc/meminfo` and the huge
//!   page sizes.
//! - [`disks`], [`net`], [`pci`]: the devices of sysfs.
//! - [`place`]: the slots and the PCI functions on the way to a device,
//!   which a location code is made of.

mod cpu;
mod disks;
mod memory;
mod net;
mod pci;
mod place;

use state::Inventory;

use crate::sysfs::text;
use crate::{Roots, smbios, system};

/// The whole inventory.
pub(crate) fn inventory(roots: &Roots, kept_serial: Option<&str>) -> Inventory {
    let sys = &roots.sys;
    let table = smbios::table(sys);
    let slots = place::slots(sys, &table);
    let cpuinfo = text(roots.proc.join("cpuinfo")).unwrap_or_default();
    let meminfo = text(roots.proc.join("meminfo")).unwrap_or_default();
    Inventory {
        system: system::read(roots, &table, kept_serial),
        processors: cpu::parse(&cpuinfo),
        memory: memory::read(sys, &meminfo),
        disks: disks::read(sys, &slots),
        interfaces: net::read(sys, &slots),
        pci: pci::read(sys, &slots),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fixture::{Tree, slot, structure};

    /// A small host: one socket, a slotted network card behind a root
    /// port, and a virtio disk.
    fn host(tree: &Tree) {
        tree.file(
            "proc/cpuinfo",
            "processor\t: 0\nmodel name\t: Example CPU\nphysical id\t: 0\ncore id\t: 0\nflags\t\t: fpu vmx\n",
        );
        tree.file("proc/meminfo", "MemTotal:        2097152 kB\n");
        tree.dir("sys/kernel/mm/hugepages/hugepages-2048kB");
        let mut table = structure(1, &[1, 2, 0, 3], &["Quartz", "Model 9", "SN12345"]);
        table.extend(slot("PCIe Slot 3", 0, 0, 0x1c, 0));
        table.extend(structure(11, &[1], &["qsvs:test"]));
        tree.file("sys/firmware/dmi/tables/DMI", &table);
        let port = "sys/devices/pci0000:00/0000:00:1c.0";
        let card = format!("{port}/0000:03:00.0");
        let disk = "sys/devices/pci0000:00/0000:00:04.0";
        for (dir, vendor, device, class) in [
            (port, "0x8086", "0x9d10", "0x060400"),
            (card.as_str(), "0x8086", "0x1533", "0x020000"),
            (disk, "0x1af4", "0x1001", "0x010000"),
        ] {
            tree.file(&format!("{dir}/vendor"), format!("{vendor}\n"));
            tree.file(&format!("{dir}/device"), format!("{device}\n"));
            tree.file(&format!("{dir}/class"), format!("{class}\n"));
            let address = dir.rsplit('/').next().unwrap();
            tree.link(&format!("sys/bus/pci/devices/{address}"), dir);
        }
        tree.link(&format!("{card}/driver"), "sys/bus/pci/drivers/igb");
        let eth = format!("{card}/net/eth0");
        tree.file(&format!("{eth}/ifindex"), "2\n");
        tree.file(&format!("{eth}/type"), "1\n");
        tree.file(&format!("{eth}/address"), "52:54:00:12:34:56\n");
        tree.file(&format!("{eth}/carrier"), "1\n");
        tree.file(&format!("{eth}/speed"), "1000\n");
        tree.link(&format!("{eth}/device"), &card);
        tree.link("sys/class/net/eth0", &eth);
        let vda = format!("{disk}/virtio1/block/vda");
        tree.file(&format!("{vda}/size"), "2097152\n");
        tree.file(&format!("{vda}/serial"), "DISK0\n");
        tree.file(&format!("{vda}/queue/rotational"), "0\n");
        tree.link("sys/class/block/vda", &vda);
    }

    #[test]
    fn a_whole_host_reads_into_one_inventory() {
        let tree = Tree::new("read-host");
        host(&tree);
        let inv = inventory(&tree.roots(), None);
        assert_eq!(inv.system.manufacturer, "Quartz");
        assert_eq!(inv.system.serial, "SN12345");
        assert_eq!(inv.system.serial_source, "dmi");
        assert_eq!(inv.processors.threads, 1);
        assert_eq!(inv.processors.virtualization, "VMX");
        assert_eq!(inv.memory.total_bytes, 2 << 30);
        assert!(inv.memory.huge_2m && !inv.memory.huge_1g);
        assert_eq!(inv.disks.len(), 1);
        assert_eq!(inv.disks[0].location, "P0000-B00-D04-F0");
        assert_eq!(inv.interfaces.len(), 1);
        assert_eq!(inv.interfaces[0].driver, "igb");
        assert_eq!(inv.interfaces[0].location, "SPCIE-SLOT-3-E0");
        let codes: Vec<&str> = inv.pci.iter().map(|p| p.location.as_str()).collect();
        assert_eq!(
            codes,
            ["P0000-B00-D04-F0", "P0000-B00-D1C-F0", "SPCIE-SLOT-3"]
        );
    }

    #[test]
    fn two_reads_of_unchanged_hardware_give_the_same_inventory() {
        let tree = Tree::new("read-stable");
        host(&tree);
        let first = inventory(&tree.roots(), None);
        assert_eq!(inventory(&tree.roots(), None), first);
    }

    #[test]
    fn the_inventory_renders_as_a_state_document() {
        let tree = Tree::new("read-render");
        host(&tree);
        let inv = inventory(&tree.roots(), None);
        let text = state::store::render(&inv).unwrap();
        assert_eq!(state::store::parse::<Inventory>(&text).unwrap(), inv);
    }
}
