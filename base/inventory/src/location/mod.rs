// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Location codes: one string that says where a disk, a network
//! interface or a PCI device is. `docs/LOCATION.md` is the format.
//!
//! The functions here are pure: the caller reads the PCI functions on
//! the way to a device from its sysfs path and the slots from SMBIOS.

use std::fmt;

/// What starts the code of a device no PCI function leads to.
pub const NO_PCI: &str = "NOPCI";

/// A PCI function's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PciAddress {
    /// The segment (the kernel's domain).
    pub segment: u32,
    /// The bus.
    pub bus: u8,
    /// The device, 0 to 31.
    pub device: u8,
    /// The function, 0 to 7.
    pub function: u8,
}

impl PciAddress {
    /// From the kernel's form, `0000:00:1f.2`.
    pub fn parse(text: &str) -> Option<Self> {
        let (segment, rest) = text.split_once(':')?;
        let (bus, rest) = rest.split_once(':')?;
        let (device, function) = rest.split_once('.')?;
        let hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
        let sized = (4..=8).contains(&segment.len())
            && bus.len() == 2
            && device.len() == 2
            && function.len() == 1;
        if !sized || ![segment, bus, device, function].into_iter().all(hex) {
            return None;
        }
        let address = PciAddress {
            segment: u32::from_str_radix(segment, 16).ok()?,
            bus: u8::from_str_radix(bus, 16).ok()?,
            device: u8::from_str_radix(device, 16).ok()?,
            function: u8::from_str_radix(function, 16).ok()?,
        };
        (address.device < 32 && address.function < 8).then_some(address)
    }

    /// Whether `other` is a function of the same device.
    pub fn same_device(self, other: PciAddress) -> bool {
        (self.segment, self.bus, self.device) == (other.segment, other.bus, other.device)
    }

    /// The code from the address alone: `P0000-B00-D1F-F2`.
    pub fn code(self) -> String {
        format!(
            "P{:04X}-B{:02X}-D{:02X}-F{}",
            self.segment, self.bus, self.device, self.function
        )
    }
}

impl fmt::Display for PciAddress {
    /// The kernel's form, `0000:00:1f.2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04x}:{:02x}:{:02x}.{:x}",
            self.segment, self.bus, self.device, self.function
        )
    }
}

/// A slot the firmware describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The designation, already through [`normalise`]; never empty.
    pub designation: String,
    /// The PCI function the firmware names for the slot.
    pub address: PciAddress,
    /// Whether that function is a bridge: the slot is then what is
    /// behind it, not the function itself.
    pub bridge: bool,
}

/// What tells several units behind one PCI function apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// The function is the unit.
    Whole,
    /// A SCSI device. The host is counted among the function's own
    /// hosts, from 0, so another adapter does not move it.
    Scsi {
        /// The host, counted within the PCI function.
        host: u32,
        /// The channel.
        channel: u32,
        /// The target.
        target: u32,
        /// The logical unit.
        lun: u64,
    },
    /// A namespace of a non-volatile memory controller.
    Namespace(u32),
    /// A port of a network adapter.
    Port(u32),
}

impl Unit {
    /// The suffix: `-T0.0.1.0`, `-N1`, `-E0`, or nothing.
    pub fn suffix(self) -> String {
        match self {
            Unit::Whole => String::new(),
            Unit::Scsi {
                host,
                channel,
                target,
                lun,
            } => format!("-T{host}.{channel}.{target}.{lun}"),
            Unit::Namespace(id) => format!("-N{id}"),
            Unit::Port(port) => format!("-E{port}"),
        }
    }
}

/// A name as a code spells it: upper case, `A-Z` and `0-9` kept, every
/// run of anything else one hyphen, none at either end.
pub fn normalise(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_uppercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// The code of a PCI function. `chain` is the PCI functions on the way
/// to it, the root's side first and the function itself last; `None`
/// when it is empty.
pub fn pci_code(chain: &[PciAddress], slots: &[Slot]) -> Option<String> {
    let (device, ancestors) = chain.split_last()?;
    // A slot that names the device itself, or another function of it.
    if let Some(slot) = slots
        .iter()
        .find(|s| !s.bridge && s.address.same_device(*device))
    {
        let tail = if slot.address.function == device.function {
            String::new()
        } else {
            format!("-F{}", device.function)
        };
        return Some(format!("S{}{tail}", slot.designation));
    }
    // A slot behind a bridge: the nearest one wins.
    for (depth, ancestor) in ancestors.iter().enumerate().rev() {
        let Some(slot) = slots.iter().find(|s| s.bridge && s.address == *ancestor) else {
            continue;
        };
        let direct = depth + 1 == ancestors.len() && device.device == 0;
        let tail = match (direct, device.function) {
            (true, 0) => String::new(),
            (true, function) => format!("-F{function}"),
            (false, _) => format!(
                "-B{:02X}-D{:02X}-F{}",
                device.bus, device.device, device.function
            ),
        };
        return Some(format!("S{}{tail}", slot.designation));
    }
    Some(device.code())
}

/// The code of a disk or a network interface: its PCI function's code
/// and the unit's suffix, or `NOPCI-<NAME>` when no PCI function leads
/// to it.
pub fn unit_code(chain: &[PciAddress], slots: &[Slot], name: &str, unit: Unit) -> String {
    match pci_code(chain, slots) {
        Some(code) => format!("{code}{}", unit.suffix()),
        None => format!("{NO_PCI}-{}", normalise(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> PciAddress {
        PciAddress::parse(text).unwrap()
    }

    fn slot(designation: &str, address: &str, bridge: bool) -> Slot {
        Slot {
            designation: normalise(designation),
            address: at(address),
            bridge,
        }
    }

    fn code(chain: &[&str], slots: &[Slot]) -> String {
        let chain: Vec<PciAddress> = chain.iter().map(|a| at(a)).collect();
        pci_code(&chain, slots).unwrap()
    }

    #[test]
    fn an_address_parses_and_prints_in_the_kernels_form() {
        let a = at("0000:00:1f.2");
        assert_eq!((a.segment, a.bus, a.device, a.function), (0, 0, 0x1f, 2));
        assert_eq!(a.to_string(), "0000:00:1f.2");
        assert_eq!(at("10000:E1:00.0").to_string(), "10000:e1:00.0");
        for bad in [
            "",
            "00:1f.2",
            "0000:00:1f",
            "0000:00:1f.8",
            "0000:00:20.0",
            "0000:0:1f.2",
            "000g:00:1f.2",
            "+000:00:1f.2",
            "2:0:0:0",
            "1-1:1.0",
            "pci0000:00",
        ] {
            assert_eq!(PciAddress::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_designation_becomes_upper_case_with_single_hyphens() {
        assert_eq!(normalise("PCIe Slot 3"), "PCIE-SLOT-3");
        assert_eq!(normalise("  J6B2 -- (x16) "), "J6B2-X16");
        assert_eq!(normalise("M.2_1"), "M-2-1");
        assert_eq!(normalise("slot\u{2116}7"), "SLOT-7");
        assert_eq!(normalise(" ... "), "");
    }

    #[test]
    fn a_function_no_slot_names_is_coded_by_its_address() {
        assert_eq!(code(&["0000:00:1f.2"], &[]), "P0000-B00-D1F-F2");
        let other = [slot("Slot 1", "0000:05:00.0", false)];
        assert_eq!(
            code(&["0000:00:01.0", "0001:af:0a.7"], &other),
            "P0001-BAF-D0A-F7"
        );
        assert_eq!(pci_code(&[], &other), None);
    }

    #[test]
    fn a_slot_that_names_the_device_covers_all_its_functions() {
        let slots = [slot("PCIe Slot 3", "0000:03:00.0", false)];
        let port = "0000:00:01.0";
        assert_eq!(code(&[port, "0000:03:00.0"], &slots), "SPCIE-SLOT-3");
        assert_eq!(code(&[port, "0000:03:00.1"], &slots), "SPCIE-SLOT-3-F1");
        // Another device on the bus is not in the slot.
        assert_eq!(code(&[port, "0000:03:01.0"], &slots), "P0000-B03-D01-F0");
    }

    #[test]
    fn a_slot_that_names_a_bridge_covers_what_is_behind_it() {
        let slots = [slot("PCIe Slot 3", "0000:00:1c.0", true)];
        let port = "0000:00:1c.0";
        // The bridge is not in its own slot, nor is the port beside it.
        assert_eq!(code(&[port], &slots), "P0000-B00-D1C-F0");
        assert_eq!(code(&["0000:00:1c.1"], &slots), "P0000-B00-D1C-F1");
        assert_eq!(code(&[port, "0000:03:00.0"], &slots), "SPCIE-SLOT-3");
        assert_eq!(code(&[port, "0000:03:00.1"], &slots), "SPCIE-SLOT-3-F1");
        // Behind a switch on the card.
        let deep = [port, "0000:03:00.0", "0000:04:01.0", "0000:06:00.0"];
        assert_eq!(code(&deep, &slots), "SPCIE-SLOT-3-B06-D00-F0");
        assert_eq!(code(&deep[..3], &slots), "SPCIE-SLOT-3-B04-D01-F0");
    }

    #[test]
    fn the_nearest_slot_wins() {
        let slots = [
            slot("Riser 1", "0000:00:01.0", true),
            slot("Riser 1 Slot 2", "0000:02:08.0", true),
            slot("Card", "0000:09:00.0", false),
        ];
        let riser = "0000:00:01.0";
        let inner = "0000:02:08.0";
        assert_eq!(
            code(&[riser, "0000:01:00.0", inner, "0000:05:00.0"], &slots),
            "SRISER-1-SLOT-2"
        );
        assert_eq!(
            code(&[riser, "0000:01:00.0", inner], &slots),
            "SRISER-1-B02-D08-F0"
        );
        // A slot that names the device comes before one above it.
        assert_eq!(code(&[riser, "0000:09:00.0"], &slots), "SCARD");
    }

    #[test]
    fn a_unit_adds_its_suffix_to_the_function_code() {
        let chain = [at("0000:00:1f.2")];
        let scsi = Unit::Scsi {
            host: 1,
            channel: 0,
            target: 2,
            lun: 3,
        };
        assert_eq!(
            unit_code(&chain, &[], "sda", scsi),
            "P0000-B00-D1F-F2-T1.0.2.3"
        );
        assert_eq!(
            unit_code(&chain, &[], "nvme0n1", Unit::Namespace(1)),
            "P0000-B00-D1F-F2-N1"
        );
        assert_eq!(
            unit_code(&chain, &[], "eth0", Unit::Port(0)),
            "P0000-B00-D1F-F2-E0"
        );
        assert_eq!(
            unit_code(&chain, &[], "vda", Unit::Whole),
            "P0000-B00-D1F-F2"
        );
        let slots = [slot("NIC 1", "0000:00:1f.2", false)];
        assert_eq!(
            unit_code(&chain, &slots, "eth0", Unit::Port(1)),
            "SNIC-1-E1"
        );
    }

    #[test]
    fn a_device_with_no_pci_function_says_so() {
        assert_eq!(unit_code(&[], &[], "mmcblk0", Unit::Whole), "NOPCI-MMCBLK0");
        assert_eq!(unit_code(&[], &[], "eth0", Unit::Port(0)), "NOPCI-ETH0");
    }

    #[test]
    fn a_code_is_upper_case_with_no_spaces() {
        let slots = [slot("pcie slot 3", "0000:00:1c.0", true)];
        let chain = [at("0000:00:1c.0"), at("0000:0a:00.1")];
        for code in [
            unit_code(&chain, &slots, "eth0", Unit::Port(0)),
            unit_code(&chain[1..], &[], "eth0", Unit::Port(0)),
            unit_code(&[], &[], "eth 0", Unit::Port(0)),
        ] {
            assert!(!code.contains(' '), "{code}");
            assert_eq!(code, code.to_uppercase());
        }
    }
}
