// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The network interfaces, from `/sys/class/net`: the ones a device
//! backs, so not the loopback, a bridge, a tap or a sub-interface.

use std::path::Path;

use state::doc::inventory::Interface;

use super::place::chain;
use crate::location::{Slot, Unit, unit_code};
use crate::sysfs::{link_name, names, number, resolve, text};

/// The network interfaces, by interface index.
pub(crate) fn read(sys: &Path, slots: &[Slot]) -> Vec<Interface> {
    let class = sys.join("class/net");
    let mut found: Vec<(u32, Interface)> = names(&class)
        .into_iter()
        .filter_map(|name| {
            let dir = class.join(&name);
            if !dir.join("device").exists() {
                return None;
            }
            let index = number(dir.join("ifindex")).unwrap_or(u32::MAX);
            let port = number(dir.join("dev_port")).unwrap_or(0);
            let chain = chain(&resolve(sys, &dir).parts);
            let interface = Interface {
                mac: text(dir.join("address"))
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                driver: link_name(dir.join("device/driver")).unwrap_or_default(),
                // The kernel says -1, or refuses the read, with no link.
                speed_mbps: number::<i64>(dir.join("speed"))
                    .filter(|speed| *speed > 0)
                    .and_then(|speed| u32::try_from(speed).ok()),
                carrier: text(dir.join("carrier")).as_deref() == Some("1"),
                location: unit_code(&chain, slots, &name, Unit::Port(port)),
                name,
            };
            Some((index, interface))
        })
        .collect();
    // Names are already in order, and the sort keeps it for equal indexes.
    found.sort_by_key(|(index, _)| *index);
    found.into_iter().map(|(_, interface)| interface).collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fixture::Tree;

    /// A NIC under `parent` (below `sys/devices`), bound to `driver`.
    fn nic(tree: &Tree, parent: &str, name: &str, index: u32, mac: &str, driver: &str) -> String {
        let device = format!("sys/devices/{parent}");
        let dir = format!("{device}/net/{name}");
        tree.file(&format!("{dir}/ifindex"), format!("{index}\n"));
        tree.file(&format!("{dir}/type"), "1\n");
        tree.file(&format!("{dir}/address"), format!("{mac}\n"));
        tree.link(&format!("{dir}/device"), &device);
        let _ = std::fs::remove_file(tree.sys().join(format!("devices/{parent}/driver")));
        tree.link(
            &format!("{device}/driver"),
            &format!("sys/bus/x/drivers/{driver}"),
        );
        tree.link(&format!("sys/class/net/{name}"), &dir);
        dir
    }

    #[test]
    fn a_nic_with_carrier_and_one_without_are_read_by_index() {
        let tree = Tree::new("net-carrier");
        let up = "pci0000:00/0000:00:02.0/virtio0";
        let down = "pci0000:00/0000:00:19.0";
        let eth1 = nic(&tree, down, "eth1", 3, "00:1B:21:AA:BB:CC", "e1000e");
        let eth0 = nic(&tree, up, "eth0", 2, "52:54:00:12:34:56", "virtio_net");
        tree.file(&format!("{eth0}/carrier"), "1\n");
        tree.file(&format!("{eth0}/speed"), "10000\n");
        // A link that is down: no carrier, and a speed of -1.
        tree.file(&format!("{eth1}/carrier"), "0\n");
        tree.file(&format!("{eth1}/speed"), "-1\n");
        // The loopback and a bridge have no device.
        tree.file("sys/class/net/lo/ifindex", "1\n");
        tree.file("sys/class/net/br0/ifindex", "4\n");
        let found = read(&tree.sys(), &[]);
        assert_eq!(
            found,
            [
                Interface {
                    name: "eth0".into(),
                    mac: "52:54:00:12:34:56".into(),
                    driver: "virtio_net".into(),
                    speed_mbps: Some(10000),
                    carrier: true,
                    location: "P0000-B00-D02-F0-E0".into(),
                },
                Interface {
                    name: "eth1".into(),
                    mac: "00:1b:21:aa:bb:cc".into(),
                    driver: "e1000e".into(),
                    speed_mbps: None,
                    carrier: false,
                    location: "P0000-B00-D19-F0-E0".into(),
                },
            ]
        );
    }

    #[test]
    fn an_unreadable_carrier_and_speed_read_as_no_link() {
        let tree = Tree::new("net-unreadable");
        nic(
            &tree,
            "pci0000:00/0000:00:19.0",
            "eth0",
            2,
            "00:1b:21:aa:bb:cc",
            "igb",
        );
        let found = read(&tree.sys(), &[]);
        assert!(!found[0].carrier);
        assert_eq!(found[0].speed_mbps, None);
    }

    #[test]
    fn two_ports_of_one_function_are_told_apart_by_the_port() {
        let tree = Tree::new("net-ports");
        let card = "pci0000:00/0000:00:01.0/0000:04:00.0";
        let a = nic(&tree, card, "eth0", 2, "00:02:c9:00:00:01", "mlx4_core");
        let b = nic(&tree, card, "eth1", 3, "00:02:c9:00:00:02", "mlx4_core");
        tree.file(&format!("{a}/dev_port"), "0\n");
        tree.file(&format!("{b}/dev_port"), "1\n");
        let found = read(&tree.sys(), &[]);
        assert_eq!(found[0].location, "P0000-B04-D00-F0-E0");
        assert_eq!(found[1].location, "P0000-B04-D00-F0-E1");
    }

    #[test]
    fn a_nic_with_no_pci_function_says_so() {
        let tree = Tree::new("net-nopci");
        nic(
            &tree,
            "platform/soc/ethernet",
            "end0",
            2,
            "02:00:00:00:00:01",
            "stmmac",
        );
        let found = read(&tree.sys(), &[]);
        assert_eq!(found[0].location, "NOPCI-END0");
        assert_eq!(found[0].driver, "stmmac");
    }
}
