// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The PCI devices, from `/sys/bus/pci/devices`: what each is, where,
//! and what physical I/O needs of it: its isolation group, how it can
//! be reset, the interrupts it can signal.

use std::fs;
use std::path::Path;

use state::doc::inventory::{Iommu, Pci};

use super::place::chain;
use crate::location::{PciAddress, Slot, pci_code};
use crate::sysfs::{hex, link_name, names, number, resolve, text};

/// The PCI devices, by address.
pub(crate) fn read(sys: &Path, slots: &[Slot]) -> Vec<Pci> {
    let devices = sys.join("bus/pci/devices");
    let mut found: Vec<(PciAddress, Pci)> = names(&devices)
        .into_iter()
        .filter_map(|name| {
            let address = PciAddress::parse(&name)?;
            let dir = devices.join(&name);
            // The functions above it, when the entry links into the tree.
            let mut chain = chain(&resolve(sys, &dir).parts);
            if chain.last() != Some(&address) {
                chain = vec![address];
            }
            let device = Pci {
                address: address.to_string(),
                vendor: hex(dir.join("vendor"), 4),
                device: hex(dir.join("device"), 4),
                class: hex(dir.join("class"), 6),
                driver: link_name(dir.join("driver")),
                location: pci_code(&chain, slots).unwrap_or_default(),
                group: link_name(dir.join("iommu_group")).and_then(|g| g.parse().ok()),
                reset: text(dir.join("reset_method"))
                    .map(|t| t.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
                interrupts: fs::read(dir.join("config"))
                    .map(|c| interrupt_kinds(&c))
                    .unwrap_or_default(),
                boot_display: number::<u32>(dir.join("boot_vga")) == Some(1),
            };
            Some((address, device))
        })
        .collect();
    found.sort_by_key(|(address, _)| *address);
    found.into_iter().map(|(_, device)| device).collect()
}

/// The capability IDs of MSI and MSI-X.
const CAP_MSI: u8 = 0x05;
const CAP_MSIX: u8 = 0x11;
/// The most capabilities a list is followed through.
const MOST_CAPABILITIES: usize = 48;

/// The interrupts a function's configuration space says it can signal:
/// `msi` and `msix` from its capability list, `intx` when its interrupt
/// pin is set. Configuration space past the header is readable by root
/// only; a shorter read lists the pin alone.
pub fn interrupt_kinds(config: &[u8]) -> Vec<String> {
    let mut kinds = Vec::new();
    let byte = |at: usize| config.get(at).copied();
    // The status register's capability list bit.
    if byte(0x06).is_some_and(|s| s & 0x10 != 0) {
        let mut at = byte(0x34).map_or(0, |p| usize::from(p & 0xFC));
        let mut seen = 0;
        while at >= 0x40 && seen < MOST_CAPABILITIES {
            let (Some(id), Some(next)) = (byte(at), byte(at + 1)) else {
                break;
            };
            let kind = match id {
                CAP_MSI => Some("msi"),
                CAP_MSIX => Some("msix"),
                _ => None,
            };
            if let Some(kind) = kind
                && !kinds.iter().any(|k| k == kind)
            {
                kinds.push(kind.to_string());
            }
            at = usize::from(next & 0xFC);
            seen += 1;
        }
    }
    if byte(0x3D).is_some_and(|pin| pin != 0) {
        kinds.push("intx".into());
    }
    kinds
}

/// The IOMMU: DMA remapping when the kernel has an IOMMU unit, and
/// interrupt remapping when its interrupt domains are the remapped ones
/// (`IR-` in `/proc/interrupts`).
pub(crate) fn iommu(sys: &Path, proc: &Path) -> Iommu {
    let translation = !names(sys.join("class/iommu")).is_empty()
        && !names(sys.join("kernel/iommu_groups")).is_empty();
    let interrupts = text(proc.join("interrupts")).unwrap_or_default();
    Iommu {
        translation,
        remapping: remapped(&interrupts),
    }
}

/// Whether `/proc/interrupts` shows remapped interrupt domains.
pub fn remapped(interrupts: &str) -> bool {
    interrupts.lines().any(|l| {
        l.split_whitespace()
            .any(|w| w.starts_with("IR-IO-APIC") || w.starts_with("IR-PCI-MSI"))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fixture::Tree;
    use crate::location::normalise;

    fn device(tree: &Tree, path: &str, ids: [&str; 3], driver: Option<&str>) {
        let dir = format!("sys/devices/{path}");
        tree.file(&format!("{dir}/vendor"), format!("{}\n", ids[0]));
        tree.file(&format!("{dir}/device"), format!("{}\n", ids[1]));
        tree.file(&format!("{dir}/class"), format!("{}\n", ids[2]));
        if let Some(driver) = driver {
            let target = format!("sys/bus/pci/drivers/{driver}");
            tree.link(&format!("{dir}/driver"), &target);
        }
        let address = path.rsplit('/').next().unwrap();
        tree.link(&format!("sys/bus/pci/devices/{address}"), &dir);
    }

    /// A configuration space with the capabilities `caps` in order from
    /// 0x40, and the interrupt pin `pin`.
    fn config(caps: &[u8], pin: u8) -> Vec<u8> {
        let mut c = vec![0u8; 256];
        if !caps.is_empty() {
            c[0x06] = 0x10;
            c[0x34] = 0x40;
        }
        for (i, id) in caps.iter().enumerate() {
            let at = 0x40 + i * 0x10;
            c[at] = *id;
            c[at + 1] = if i + 1 < caps.len() {
                u8::try_from(at + 0x10).unwrap()
            } else {
                0
            };
        }
        c[0x3D] = pin;
        c
    }

    #[test]
    fn devices_are_read_with_and_without_a_driver_by_address() {
        let tree = Tree::new("pci-devices");
        let ahci = ["0x8086", "0x2922", "0x010601"];
        let bridge = ["0x1B36", "0x000c", "0x060400"];
        let net = ["0x1af4", "0x1041", "0x020000"];
        device(&tree, "pci0000:00/0000:00:1f.2", ahci, Some("ahci"));
        device(&tree, "pci0000:00/0000:00:02.0", bridge, Some("pcieport"));
        device(&tree, "pci0000:00/0000:00:02.0/0000:01:00.0", net, None);
        tree.dir("sys/bus/pci/devices/not-an-address");
        let found = read(&tree.sys(), &[]);
        let addresses: Vec<&str> = found.iter().map(|p| p.address.as_str()).collect();
        assert_eq!(addresses, ["0000:00:02.0", "0000:00:1f.2", "0000:01:00.0"]);
        assert_eq!(
            found[1],
            Pci {
                address: "0000:00:1f.2".into(),
                vendor: "8086".into(),
                device: "2922".into(),
                class: "010601".into(),
                driver: Some("ahci".into()),
                location: "P0000-B00-D1F-F2".into(),
                ..Pci::default()
            }
        );
        assert_eq!(found[0].vendor, "1b36");
        assert_eq!(found[0].driver.as_deref(), Some("pcieport"));
        assert_eq!(found[2].driver, None);
        assert_eq!(found[2].location, "P0000-B01-D00-F0");
    }

    #[test]
    fn the_group_resets_interrupts_and_display_are_read() {
        let tree = Tree::new("pci-physio");
        let nvme = ["0x1b36", "0x0010", "0x010802"];
        let vga = ["0x1234", "0x1111", "0x030000"];
        device(&tree, "pci0000:00/0000:00:04.0", nvme, Some("nvme"));
        device(&tree, "pci0000:00/0000:00:01.0", vga, None);
        let dir = "sys/devices/pci0000:00/0000:00:04.0";
        tree.link(&format!("{dir}/iommu_group"), "sys/kernel/iommu_groups/7");
        tree.file(&format!("{dir}/reset_method"), "flr bus\n");
        tree.file(
            &format!("{dir}/config"),
            config(&[0x01, CAP_MSIX, CAP_MSI], 1),
        );
        tree.file("sys/devices/pci0000:00/0000:00:01.0/boot_vga", "1\n");
        let found = read(&tree.sys(), &[]);
        assert_eq!(found[1].group, Some(7));
        assert_eq!(found[1].reset, ["flr", "bus"]);
        assert_eq!(found[1].interrupts, ["msix", "msi", "intx"]);
        assert!(!found[1].boot_display);
        assert_eq!(found[0].group, None);
        assert!(found[0].reset.is_empty());
        assert!(found[0].boot_display);
    }

    #[test]
    fn interrupt_kinds_follow_the_list_and_stop_on_a_loop_or_a_short_read() {
        assert_eq!(interrupt_kinds(&config(&[CAP_MSI], 0)), ["msi"]);
        assert_eq!(interrupt_kinds(&config(&[], 2)), ["intx"]);
        // A short read: the header's pin alone.
        assert_eq!(interrupt_kinds(&config(&[CAP_MSIX], 1)[..64]), ["intx"]);
        // A capability that points at itself is followed a bounded time.
        let mut looped = config(&[CAP_MSIX], 0);
        looped[0x41] = 0x40;
        assert_eq!(interrupt_kinds(&looped), ["msix"]);
        // A pointer into the header ends the list.
        let mut low = config(&[CAP_MSI], 0);
        low[0x34] = 0x20;
        assert!(interrupt_kinds(&low).is_empty());
        assert!(interrupt_kinds(&[]).is_empty());
    }

    #[test]
    fn remapping_is_seen_in_the_interrupt_domains() {
        let on = "  0:  12  IR-IO-APIC  2-edge  timer\n 24:  0  IR-PCI-MSIX-0000:00:04.0  0-edge  nvme0q0\n";
        let off =
            "  0:  12  IO-APIC  2-edge  timer\n 24:  0  PCI-MSIX-0000:00:04.0  0-edge  nvme0q0\n";
        assert!(remapped(on));
        assert!(!remapped(off));
        assert!(!remapped(""));
    }

    #[test]
    fn the_iommu_needs_a_unit_and_groups() {
        let tree = Tree::new("pci-iommu");
        assert_eq!(
            iommu(&tree.sys(), &tree.roots().proc),
            Iommu {
                translation: false,
                remapping: false
            }
        );
        tree.dir("sys/class/iommu/dmar0");
        tree.dir("sys/kernel/iommu_groups/0");
        tree.file("proc/interrupts", "  0:  1  IR-IO-APIC  2-edge  timer\n");
        assert_eq!(
            iommu(&tree.sys(), &tree.roots().proc),
            Iommu {
                translation: true,
                remapping: true
            }
        );
    }

    #[test]
    fn a_device_behind_a_slotted_bridge_takes_the_slots_code() {
        let tree = Tree::new("pci-slot");
        let bridge = ["0x8086", "0x9d10", "0x060400"];
        let net = ["0x8086", "0x1533", "0x020000"];
        device(&tree, "pci0000:00/0000:00:1c.0", bridge, None);
        device(&tree, "pci0000:00/0000:00:1c.0/0000:03:00.0", net, None);
        device(&tree, "pci0000:00/0000:00:1c.0/0000:03:00.1", net, None);
        let slots = [Slot {
            designation: normalise("PCIe Slot 3"),
            address: PciAddress::parse("0000:00:1c.0").unwrap(),
            bridge: true,
        }];
        let found = read(&tree.sys(), &slots);
        let codes: Vec<&str> = found.iter().map(|p| p.location.as_str()).collect();
        assert_eq!(
            codes,
            ["P0000-B00-D1C-F0", "SPCIE-SLOT-3", "SPCIE-SLOT-3-F1"]
        );
    }

    #[test]
    fn a_device_that_is_not_a_link_is_coded_by_its_own_address() {
        let tree = Tree::new("pci-plain");
        tree.file("sys/bus/pci/devices/0000:00:03.0/vendor", "0x1af4\n");
        let found = read(&tree.sys(), &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].vendor, "1af4");
        assert_eq!(found[0].device, "");
        assert_eq!(found[0].location, "P0000-B00-D03-F0");
    }
}
