// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `inventory.toml`: what the host is made of, rewritten at every IPL.
//! The product reads the hardware; this is the file it fills.

use serde::{Deserialize, Serialize};

use crate::store::{Document, Migration};

/// The system as the firmware names it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct System {
    /// The manufacturer.
    #[serde(default)]
    pub manufacturer: String,
    /// The product name.
    #[serde(default)]
    pub product: String,
    /// The serial number: DMI's, or the one kept in `host.toml`.
    #[serde(default)]
    pub serial: String,
    /// Where the serial number came from: `dmi` or `derived`.
    #[serde(default)]
    pub serial_source: String,
}

/// The processors.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Processors {
    /// Populated sockets.
    pub sockets: u32,
    /// Cores, over all sockets.
    pub cores: u32,
    /// Threads, over all cores.
    pub threads: u32,
    /// The model name of the first processor.
    pub model: String,
    /// `VMX`, `SVM` or `none`.
    pub virtualization: String,
}

/// The memory.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    /// Bytes the kernel has.
    pub total_bytes: u64,
    /// Whether 2 MB huge pages are available.
    pub huge_2m: bool,
    /// Whether 1 GB huge pages are available.
    pub huge_1g: bool,
}

/// One disk.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    /// The kernel's name, `nvme0n1`.
    pub name: String,
    /// The model.
    pub model: String,
    /// The serial number.
    pub serial: String,
    /// Its size.
    pub bytes: u64,
    /// Whether it spins.
    pub rotational: bool,
    /// `nvme`, `sata`, `sas`, `scsi`, `virtio`, `usb` or `unknown`.
    pub transport: String,
    /// The location code.
    pub location: String,
}

/// One network interface.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// The kernel's name.
    pub name: String,
    /// The hardware address.
    pub mac: String,
    /// The driver.
    pub driver: String,
    /// The speed in Mb/s, when the link reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mbps: Option<u32>,
    /// Whether the link has carrier.
    pub carrier: bool,
    /// The location code.
    pub location: String,
}

/// One PCI device.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pci {
    /// `0000:00:1f.2`.
    pub address: String,
    /// The vendor ID, four hex digits.
    pub vendor: String,
    /// The device ID, four hex digits.
    pub device: String,
    /// The class code, six hex digits.
    pub class: String,
    /// The driver bound to it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    /// The location code.
    pub location: String,
}

/// `inventory.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    /// The system.
    #[serde(default)]
    pub system: System,
    /// The processors.
    #[serde(default)]
    pub processors: Processors,
    /// The memory.
    #[serde(default)]
    pub memory: Memory,
    /// The disks, by name.
    #[serde(default)]
    pub disks: Vec<Disk>,
    /// The network interfaces, by interface index.
    #[serde(default)]
    pub interfaces: Vec<Interface>,
    /// The PCI devices, by address.
    #[serde(default)]
    pub pci: Vec<Pci>,
}

impl Document for Inventory {
    const FILE: &'static str = "inventory.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{parse, render};

    #[test]
    fn it_round_trips_with_every_kind_of_device() {
        let inv = Inventory {
            system: System {
                manufacturer: "QEMU".into(),
                product: "Standard PC (Q35 + ICH9, 2009)".into(),
                serial: "M525400123456".into(),
                serial_source: "derived".into(),
            },
            processors: Processors {
                sockets: 2,
                cores: 4,
                threads: 8,
                model: "Example CPU".into(),
                virtualization: "VMX".into(),
            },
            memory: Memory {
                total_bytes: 2 << 30,
                huge_2m: true,
                huge_1g: false,
            },
            disks: vec![Disk {
                name: "vda".into(),
                model: "virtio".into(),
                serial: "DISK0".into(),
                bytes: 1 << 30,
                rotational: false,
                transport: "virtio".into(),
                location: "P0-B00-D03-F0".into(),
            }],
            interfaces: vec![Interface {
                name: "eth0".into(),
                mac: "52:54:00:12:34:56".into(),
                driver: "virtio_net".into(),
                speed_mbps: None,
                carrier: true,
                location: "P0-B00-D02-F0".into(),
            }],
            pci: vec![Pci {
                address: "0000:00:02.0".into(),
                vendor: "1af4".into(),
                device: "1000".into(),
                class: "020000".into(),
                driver: Some("virtio-pci".into()),
                location: "P0-B00-D02-F0".into(),
            }],
        };
        let text = render(&inv).unwrap();
        assert!(text.contains("[[disks]]"), "{text}");
        assert_eq!(parse::<Inventory>(&text).unwrap(), inv);
    }
}
