// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The PCI devices, from `/sys/bus/pci/devices`.

use std::path::Path;

use state::doc::inventory::Pci;

use super::place::chain;
use crate::location::{PciAddress, Slot, pci_code};
use crate::sysfs::{hex, link_name, names, resolve};

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
            };
            Some((address, device))
        })
        .collect();
    found.sort_by_key(|(address, _)| *address);
    found.into_iter().map(|(_, device)| device).collect()
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
            }
        );
        assert_eq!(found[0].vendor, "1b36");
        assert_eq!(found[0].driver.as_deref(), Some("pcieport"));
        assert_eq!(found[2].driver, None);
        assert_eq!(found[2].location, "P0000-B01-D00-F0");
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
