// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a location code is made of: the slots SMBIOS describes, and the
//! PCI functions on the way to a device in its canonical sysfs path.

use std::path::Path;

use crate::location::{PciAddress, Slot, normalise};
use crate::smbios;
use crate::sysfs::hex;

/// The class code of a PCI-to-PCI bridge starts with this.
const BRIDGE_CLASS: &str = "0604";

/// The slots of the SMBIOS `table` whose function is present. A slot
/// with a designation that normalises to nothing is left out, and so
/// are slots that share a designation: their code would not be one
/// place.
pub(crate) fn slots(sys: &Path, table: &[u8]) -> Vec<Slot> {
    let found: Vec<Slot> = smbios::slots(table)
        .into_iter()
        .filter_map(|record| {
            let address = PciAddress {
                segment: u32::from(record.segment),
                bus: record.bus,
                device: record.device,
                function: record.function,
            };
            let designation = normalise(&record.designation);
            let device = sys.join("bus/pci/devices").join(address.to_string());
            if designation.is_empty() || !device.exists() {
                return None;
            }
            Some(Slot {
                designation,
                address,
                bridge: hex(device.join("class"), 6).starts_with(BRIDGE_CLASS),
            })
        })
        .collect();
    let once = |slot: &Slot| {
        found
            .iter()
            .filter(|other| other.designation == slot.designation)
            .count()
            == 1
    };
    found.iter().filter(|slot| once(slot)).cloned().collect()
}

/// The PCI functions among the components of a sysfs path, in order.
pub(crate) fn chain(parts: &[String]) -> Vec<PciAddress> {
    parts
        .iter()
        .filter_map(|part| PciAddress::parse(part))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Tree, slot};

    #[test]
    fn the_chain_is_the_pci_functions_of_the_path() {
        let parts: Vec<String> =
            "devices/pci0000:00/0000:00:1c.0/0000:03:00.0/ata1/host0/target0:0:0/0:0:0:0/block/sda"
                .split('/')
                .map(str::to_string)
                .collect();
        let found: Vec<String> = chain(&parts).iter().map(ToString::to_string).collect();
        assert_eq!(found, ["0000:00:1c.0", "0000:03:00.0"]);
        assert!(chain(&[]).is_empty());
    }

    #[test]
    fn a_slot_is_kept_when_its_function_is_present() {
        let tree = Tree::new("place-slots");
        tree.file("sys/bus/pci/devices/0000:00:1c.0/class", "0x060400\n");
        tree.file("sys/bus/pci/devices/0000:05:00.0/class", "0x020000\n");
        let mut table = slot("PCIe Slot 3", 0, 0, 0x1c, 0);
        table.extend(slot("PCIe Slot 4", 0, 5, 0, 0));
        table.extend(slot("Empty Slot", 0, 6, 0, 0));
        table.extend(slot("...", 0, 5, 0, 0));
        let found = slots(&tree.sys(), &table);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].designation, "PCIE-SLOT-3");
        assert!(found[0].bridge);
        assert_eq!(found[1].designation, "PCIE-SLOT-4");
        assert_eq!(found[1].address.to_string(), "0000:05:00.0");
        assert!(!found[1].bridge);
        assert!(slots(&tree.sys(), &[]).is_empty());
    }

    #[test]
    fn slots_with_one_designation_are_not_used() {
        let tree = Tree::new("place-twins");
        tree.file("sys/bus/pci/devices/0000:04:00.0/class", "0x020000\n");
        tree.file("sys/bus/pci/devices/0000:05:00.0/class", "0x020000\n");
        tree.file("sys/bus/pci/devices/0000:06:00.0/class", "0x010802\n");
        let mut table = slot("PCIe Slot", 0, 4, 0, 0);
        table.extend(slot("PCIE slot", 0, 5, 0, 0));
        table.extend(slot("M.2 Slot 1", 0, 6, 0, 0));
        let found = slots(&tree.sys(), &table);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].designation, "M-2-SLOT-1");
    }
}
