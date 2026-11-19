// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host inventory, read at every IPL: `base/inventory` (the system,
//! processors, memory, disks, network interfaces and PCI devices, each
//! with a location code, `docs/LOCATION.md`) with a hypervisor's own
//! fields on top: the PCI devices' isolation groups, resets and
//! interrupts, the IOMMU, the huge page sizes, and the tape drives a
//! partition can be given.
//!
//! - [`read`]: the whole inventory, for `inventory.toml`.
//! - [`pci`], [`iommu`], [`tapes`]: what physical I/O reads.
//! - The rest is `base/inventory`'s, named here as it was.
//!
//! Nothing here fails: a node that is missing reads as empty or zero.

#![forbid(unsafe_code)]

mod huge;
mod pci;
mod tapes;

pub use inventory::{
    CpuThread, Roots, derived_serial, disks, dmi_serial, interfaces, location, memory_mib,
    oem_strings, smbios, threads,
};

/// Read the whole inventory. `kept_serial` is the serial number already
/// kept in `host.toml`, if any (`inventory::read`).
pub fn read(roots: &Roots, kept_serial: Option<&str>) -> state::Inventory {
    let mut inv = inventory::read(roots, kept_serial);
    huge::fill(&roots.sys, &mut inv.memory);
    pci::isolation(&roots.sys, &mut inv.pci);
    inv.iommu = iommu(roots);
    inv
}

/// The IOMMU: whether it translates and remaps interrupts.
pub fn iommu(roots: &Roots) -> state::doc::inventory::Iommu {
    pci::iommu(&roots.sys, &roots.proc)
}

/// The PCI devices as they are now, with their isolation groups, resets
/// and interrupts.
pub fn pci(roots: &Roots) -> Vec<state::doc::inventory::Pci> {
    let mut devices = inventory::pci(roots);
    pci::isolation(&roots.sys, &mut devices);
    devices
}

/// The interrupts a function's configuration space says it can signal.
pub use pci::{interrupt_kinds, remapped};
/// A physical tape drive.
pub use tapes::TapeDrive;

/// The physical tape drives, by their tape device's number.
pub fn tapes(roots: &Roots) -> Vec<TapeDrive> {
    tapes::read(&roots.sys, &inventory::read::slots(roots))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use inventory::fixture::{Tree, slot, structure};

    /// A small host: one socket with VMX, 2 MB pages, a slotted network
    /// card behind a root port in an isolation group, and a virtio disk.
    fn host(tree: &Tree) {
        tree.file(
            "proc/cpuinfo",
            "processor\t: 0\nmodel name\t: Example CPU\nphysical id\t: 0\ncore id\t: 0\nflags\t\t: fpu vmx\n",
        );
        tree.file("proc/meminfo", "MemTotal:        2097152 kB\n");
        tree.file("proc/interrupts", "  0:  1  IR-IO-APIC  2-edge  timer\n");
        tree.dir("sys/kernel/mm/hugepages/hugepages-2048kB");
        tree.dir("sys/class/iommu/dmar0");
        tree.dir("sys/kernel/iommu_groups/5");
        let mut table = structure(1, &[1, 2, 0, 3], &["Quartz", "Model 9", "SN12345"]);
        table.extend(slot("PCIe Slot 3", 0, 0, 0x1c, 0));
        tree.file("sys/firmware/dmi/tables/DMI", &table);
        let port = "sys/devices/pci0000:00/0000:00:1c.0";
        let card = format!("{port}/0000:03:00.0");
        for (dir, class) in [(port, "0x060400"), (card.as_str(), "0x020000")] {
            tree.file(&format!("{dir}/vendor"), "0x8086\n");
            tree.file(&format!("{dir}/device"), "0x1533\n");
            tree.file(&format!("{dir}/class"), format!("{class}\n"));
            let address = dir.rsplit('/').next().unwrap();
            tree.link(&format!("sys/bus/pci/devices/{address}"), dir);
        }
        tree.link(&format!("{card}/iommu_group"), "sys/kernel/iommu_groups/5");
        tree.file(&format!("{card}/reset_method"), "flr\n");
    }

    #[test]
    fn the_host_s_fields_are_added_to_the_base_inventory() {
        let tree = Tree::new("hostinv-host");
        host(&tree);
        let inv = read(&tree.roots(), None);
        let base = inventory::read(&tree.roots(), None);
        assert_eq!(inv.system, base.system);
        assert_eq!(inv.processors.virtualization, "VMX");
        assert_eq!(inv.memory.total_bytes, base.memory.total_bytes);
        assert!(inv.memory.huge_2m && !inv.memory.huge_1g);
        assert!(!base.memory.huge_2m);
        assert!(inv.iommu.translation && inv.iommu.remapping);
        assert_eq!(inv.pci.len(), 2);
        assert_eq!(inv.pci[1].location, "SPCIE-SLOT-3");
        assert_eq!(inv.pci[1].group, Some(5));
        assert_eq!(inv.pci[1].reset, ["flr"]);
        assert_eq!(base.pci[1].group, None);
        assert_eq!(pci(&tree.roots()), inv.pci);
    }

    #[test]
    fn the_inventory_renders_as_a_state_document() {
        let tree = Tree::new("hostinv-render");
        host(&tree);
        let inv = read(&tree.roots(), None);
        let text = state::store::render(&inv).unwrap();
        assert_eq!(state::store::parse::<state::Inventory>(&text).unwrap(), inv);
    }

    #[test]
    fn the_oem_strings_are_base_s() {
        let tree = Tree::new("hostinv-oem");
        let table = structure(11, &[2], &["qsvs:role=host", "second"]);
        tree.file("sys/firmware/dmi/tables/DMI", &table);
        assert_eq!(oem_strings(&tree.roots()), ["qsvs:role=host", "second"]);
    }
}
