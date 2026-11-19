// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The structures the inventory reads: system information (type 1),
//! system slots (type 9) and OEM strings (type 11).

use super::structures;

/// System information.
const SYSTEM: u8 = 1;
/// A system slot.
const SLOT: u8 = 9;
/// OEM strings.
const OEM_STRINGS: u8 = 11;

/// The length of a type 9 structure that carries an address (SMBIOS 2.6).
const SLOT_WITH_ADDRESS: usize = 0x11;

/// System information (type 1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemRecord {
    /// The manufacturer.
    pub manufacturer: String,
    /// The product name.
    pub product: String,
    /// The serial number, as the firmware has it.
    pub serial: String,
}

/// A system slot (type 9) that says which PCI function it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotRecord {
    /// The designation, as the firmware has it: `PCIe Slot 3`.
    pub designation: String,
    /// The segment group.
    pub segment: u16,
    /// The bus.
    pub bus: u8,
    /// The device, 0 to 31.
    pub device: u8,
    /// The function, 0 to 7.
    pub function: u8,
}

/// The first system information structure.
pub fn system(table: &[u8]) -> Option<SystemRecord> {
    let found = structures(table);
    let s = found.iter().find(|s| s.kind == SYSTEM)?;
    Some(SystemRecord {
        manufacturer: s.string_at(0x04),
        product: s.string_at(0x05),
        serial: s.string_at(0x07),
    })
}

/// The slots that carry an address. A slot from before SMBIOS 2.6, or
/// whose address is all ones, is left out.
pub fn slots(table: &[u8]) -> Vec<SlotRecord> {
    structures(table)
        .iter()
        .filter(|s| s.kind == SLOT && s.formatted.len() >= SLOT_WITH_ADDRESS)
        .filter_map(|s| {
            let segment = s.word(0x0d)?;
            let bus = s.byte(0x0f)?;
            let devfn = s.byte(0x10)?;
            if segment == 0xffff || (bus == 0xff && devfn == 0xff) {
                return None;
            }
            Some(SlotRecord {
                designation: s.string_at(0x04),
                segment,
                bus,
                device: devfn >> 3,
                function: devfn & 7,
            })
        })
        .collect()
}

/// The OEM strings, in order, over every type 11 structure.
pub fn oem_strings(table: &[u8]) -> Vec<String> {
    let mut strings = Vec::new();
    for s in structures(table).iter().filter(|s| s.kind == OEM_STRINGS) {
        let count = s.byte(0x04).unwrap_or(0);
        strings.extend((1..=count).filter_map(|index| s.string(index)));
    }
    strings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{slot, structure};

    #[test]
    fn type_1_gives_the_manufacturer_the_product_and_the_serial() {
        let mut table = structure(0, &[1], &["Firmware Vendor"]);
        // Manufacturer, product, version, serial, then the UUID.
        let mut body = vec![1, 2, 3, 4];
        body.extend([0u8; 16]);
        table.extend(structure(
            1,
            &body,
            &["Quartz", "Model 9 ", "1.0", " SN12345"],
        ));
        assert_eq!(
            system(&table),
            Some(SystemRecord {
                manufacturer: "Quartz".into(),
                product: "Model 9".into(),
                serial: "SN12345".into(),
            })
        );
    }

    #[test]
    fn type_1_with_no_strings_reads_as_empty_and_no_type_1_as_nothing() {
        let table = structure(1, &[0, 0, 0, 0], &[]);
        assert_eq!(system(&table), Some(SystemRecord::default()));
        // A structure too short to hold the serial field.
        let short = structure(1, &[1], &["Quartz"]);
        assert_eq!(system(&short).unwrap().serial, "");
        assert_eq!(system(&structure(11, &[0], &[])), None);
        assert_eq!(system(&[]), None);
    }

    #[test]
    fn type_9_gives_the_designation_and_the_address() {
        let mut table = slot("PCIe Slot 3", 0, 0x03, 0, 0);
        table.extend(slot("M.2 Slot 1", 1, 0x00, 0x1c, 4));
        assert_eq!(
            slots(&table),
            [
                SlotRecord {
                    designation: "PCIe Slot 3".into(),
                    segment: 0,
                    bus: 3,
                    device: 0,
                    function: 0,
                },
                SlotRecord {
                    designation: "M.2 Slot 1".into(),
                    segment: 1,
                    bus: 0,
                    device: 0x1c,
                    function: 4,
                },
            ]
        );
    }

    #[test]
    fn a_slot_with_no_address_is_left_out() {
        // Before SMBIOS 2.6: the structure ends at the characteristics.
        let old = [1, 0xa5, 0x0b, 0x04, 0x04, 1, 0, 0x0c, 0x01];
        let mut table = structure(9, &old, &["Old"]);
        // An address of all ones.
        table.extend(slot("Empty", 0, 0xff, 0x1f, 7));
        table.extend(slot("No segment", 0xffff, 0, 1, 0));
        assert!(slots(&table).is_empty());
    }

    #[test]
    fn type_11_gives_the_oem_strings_in_order() {
        let mut table = structure(11, &[2], &["first", "second"]);
        table.extend(structure(1, &[0, 0, 0, 0], &[]));
        // The count wins over the set.
        table.extend(structure(11, &[1], &["third", "uncounted"]));
        // A count past the set.
        table.extend(structure(11, &[3], &["fourth"]));
        assert_eq!(oem_strings(&table), ["first", "second", "third", "fourth"]);
        assert!(oem_strings(&[]).is_empty());
    }
}
