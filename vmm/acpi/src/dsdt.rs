// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The DSDT: what the guest cannot find by looking.
//!
//! | Device | Id | Resources |
//! |---|---|---|
//! | `\_SB.PCI0` | `PNP0A08`, `PNP0A03` | bus 0, the configuration ports, PCI memory space |
//! | `\_SB.MBRD` | `PNP0C02` | memory-mapped configuration space, so the kernel trusts the MCFG |
//! | `\_SB.COM1` | `PNP0501` | the 16550's ports and interrupt 4 |
//! | `\_SB.RTC0` | `PNP0B00` | the CMOS clock's ports and interrupt 8 |
//! | `\_SB.PWRD` | `PNP0C02` | the sleep control and status registers |
//!
//! `\_S5` gives the sleep type that powers the partition off.

use acpi_tables::Aml;
use acpi_tables::aml::{
    AddressSpace, AddressSpaceCacheable, Device, EISAName, IO, Interrupt, Memory32Fixed, Name, ONE,
    Package, ResourceTemplate, ZERO,
};
use acpi_tables::sdt::Sdt;
use arch::layout::{
    CMOS_IRQ, CMOS_PORT, ECAM_BYTES, ECAM_START, PCI_CONFIG_PORT, PCI_MEM_END, PCI_MEM_START,
    SERIAL_IRQ, SERIAL_PORT, SLEEP_PORT, SLEEP_TYPE_S5,
};

use crate::{OEM_ID, OEM_REVISION, OEM_TABLE_ID};

/// Bytes of a table's header.
const HEADER: u32 = 36;

#[allow(clippy::cast_possible_truncation)] // every address here is below 4 GiB
const fn low(address: u64) -> u32 {
    address as u32
}

/// One device under `\_SB`: its id, its unit and what it uses.
fn device(dsdt: &mut Sdt, path: &str, hid: &str, uid: &dyn Aml, resources: Vec<&dyn Aml>) {
    Device::new(
        path.into(),
        vec![
            &Name::new("_HID".into(), &EISAName::new(hid)),
            &Name::new("_UID".into(), uid),
            &Name::new("_CRS".into(), &ResourceTemplate::new(resources)),
        ],
    )
    .to_aml_bytes(dsdt);
}

/// The DSDT's bytes.
pub fn build() -> Vec<u8> {
    let mut dsdt = Sdt::new(*b"DSDT", HEADER, 6, OEM_ID, OEM_TABLE_ID, OEM_REVISION);

    Device::new(
        "_SB_.PCI0".into(),
        vec![
            &Name::new("_HID".into(), &EISAName::new("PNP0A08")),
            &Name::new("_CID".into(), &EISAName::new("PNP0A03")),
            &Name::new("_ADR".into(), &ZERO),
            &Name::new("_SEG".into(), &ZERO),
            &Name::new("_UID".into(), &ZERO),
            &Name::new(
                "_CRS".into(),
                &ResourceTemplate::new(vec![
                    &AddressSpace::new_bus_number(0u16, 0u16),
                    &IO::new(PCI_CONFIG_PORT, PCI_CONFIG_PORT, 1, 8),
                    &AddressSpace::new_memory(
                        AddressSpaceCacheable::NotCacheable,
                        true,
                        low(PCI_MEM_START),
                        low(PCI_MEM_END - 1),
                        None,
                    ),
                ]),
            ),
        ],
    )
    .to_aml_bytes(&mut dsdt);

    device(
        &mut dsdt,
        "_SB_.MBRD",
        "PNP0C02",
        &ZERO,
        vec![&Memory32Fixed::new(true, low(ECAM_START), low(ECAM_BYTES))],
    );
    device(
        &mut dsdt,
        "_SB_.COM1",
        "PNP0501",
        &ZERO,
        vec![
            &Interrupt::new(true, true, false, false, SERIAL_IRQ),
            &IO::new(SERIAL_PORT, SERIAL_PORT, 0, 8),
        ],
    );
    device(
        &mut dsdt,
        "_SB_.RTC0",
        "PNP0B00",
        &ZERO,
        vec![
            &IO::new(CMOS_PORT, CMOS_PORT, 0, 2),
            &Interrupt::new(true, true, false, false, CMOS_IRQ),
        ],
    );
    device(
        &mut dsdt,
        "_SB_.PWRD",
        "PNP0C02",
        &ONE,
        vec![&IO::new(SLEEP_PORT, SLEEP_PORT, 0, 2)],
    );

    Name::new(
        "_S5_".into(),
        &Package::new(vec![&SLEEP_TYPE_S5, &ZERO, &ZERO, &ZERO]),
    )
    .to_aml_bytes(&mut dsdt);

    dsdt.update_checksum();
    dsdt.as_slice().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn the_table_is_whole_and_names_its_devices() {
        let dsdt = build();
        assert_eq!(&dsdt[..4], b"DSDT");
        let length = u32::from_le_bytes(dsdt[4..8].try_into().unwrap());
        assert_eq!(length as usize, dsdt.len());
        assert_eq!(dsdt.iter().fold(0u8, |a, b| a.wrapping_add(*b)), 0);
        for name in [&b"PCI0"[..], b"MBRD", b"COM1", b"RTC0", b"PWRD", b"_S5_"] {
            assert!(has(&dsdt, name), "{}", String::from_utf8_lossy(name));
        }
        // PNP0501 as an EISA id.
        assert!(has(&dsdt, &[0x41, 0xD0, 0x05, 0x01]));
        // The serial port's ports, as an I/O descriptor.
        assert!(has(
            &dsdt,
            &[0x47, 0x01, 0xF8, 0x03, 0xF8, 0x03, 0x00, 0x08]
        ));
    }
}
