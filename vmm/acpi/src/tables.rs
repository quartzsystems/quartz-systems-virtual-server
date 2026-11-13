// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The fixed tables and the block they are laid out in: the RSDP at
//! the block's start, then the DSDT, FADT, MADT, MCFG and XSDT, each
//! on a 16-byte boundary.

use acpi_tables::madt::{
    EnabledStatus, IoApic, LocalInterruptController, MADT, ProcessorLocalApic,
};
use acpi_tables::mcfg::MCFG;
use acpi_tables::rsdp::Rsdp;
use acpi_tables::sdt::Sdt;
use acpi_tables::xsdt::XSDT;
use acpi_tables::{Aml, AmlSink};
use arch::layout::{
    ACPI_BYTES, ECAM_START, I8042_COMMAND_PORT, I8042_RESET, IOAPIC_START, LAPIC_START, SLEEP_PORT,
};

use crate::{OEM_ID, OEM_REVISION, OEM_TABLE_ID, dsdt};

/// Bytes of a FADT of ACPI 6.
const FADT_BYTES: u32 = 276;

/// FADT flags: no fixed power button, no fixed sleep button, the reset
/// register is there, hardware-reduced.
const FADT_FLAGS: u32 = (1 << 4) | (1 << 5) | (1 << 10) | (1 << 20);
/// Boot architecture flags: no VGA. No 8042 keyboard is claimed and
/// the CMOS clock is not denied.
const FADT_BOOT_ARCH: u16 = 1 << 2;

/// A generic address in I/O space, eight bits wide.
fn io_register(port: u16) -> [u8; 12] {
    let mut gas = [0u8; 12];
    gas[0] = 1; // system I/O
    gas[1] = 8; // bits
    gas[3] = 1; // byte access
    gas[4..6].copy_from_slice(&port.to_le_bytes());
    gas
}

struct Bytes(Vec<u8>);

impl AmlSink for Bytes {
    fn byte(&mut self, byte: u8) {
        self.0.push(byte);
    }
}

fn bytes_of(table: &dyn Aml) -> Vec<u8> {
    let mut sink = Bytes(Vec::new());
    table.to_aml_bytes(&mut sink);
    sink.0
}

fn fadt(dsdt_address: u64) -> Vec<u8> {
    let mut t = Sdt::new(*b"FACP", FADT_BYTES, 6, OEM_ID, OEM_TABLE_ID, OEM_REVISION);
    t.write_u16(109, FADT_BOOT_ARCH);
    t.write_u32(112, FADT_FLAGS);
    t.write_bytes(116, &io_register(I8042_COMMAND_PORT));
    t.write_u8(128, I8042_RESET);
    t.write_u8(131, 5); // FADT minor version
    t.write_u64(140, dsdt_address);
    t.write_bytes(244, &io_register(SLEEP_PORT));
    t.write_bytes(256, &io_register(SLEEP_PORT + 1));
    t.write_bytes(268, b"QUARTZVS");
    t.update_checksum();
    t.as_slice().to_vec()
}

fn madt(vcpus: u8) -> Vec<u8> {
    #[allow(clippy::cast_possible_truncation)] // both are below 4 GiB
    let (lapic, ioapic) = (LAPIC_START as u32, IOAPIC_START as u32);
    let mut t = MADT::new(
        OEM_ID,
        OEM_TABLE_ID,
        OEM_REVISION,
        LocalInterruptController::Address(lapic),
    );
    for id in 0..vcpus {
        t.add_structure(ProcessorLocalApic::new(id, id, EnabledStatus::Enabled));
    }
    // The I/O APIC's id follows the processors'.
    t.add_structure(IoApic::new(vcpus, ioapic, 0));
    bytes_of(&t)
}

fn mcfg() -> Vec<u8> {
    let mut t = MCFG::new(OEM_ID, OEM_TABLE_ID, OEM_REVISION);
    t.add_ecam(ECAM_START, 0, 0, 0);
    bytes_of(&t)
}

/// The tables as one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tables {
    /// The bytes, to be written at the address they were built for.
    pub bytes: Vec<u8>,
    /// Where each table is in the block, by signature.
    pub offsets: Vec<([u8; 4], usize)>,
}

fn place(block: &mut Vec<u8>, table: &[u8]) -> usize {
    while !block.len().is_multiple_of(16) {
        block.push(0);
    }
    let at = block.len();
    block.extend_from_slice(table);
    at
}

/// The tables of a partition with `vcpus` processors, for a block at
/// guest address `base`. `None` when they do not fit the room kept.
pub fn build(vcpus: u8, base: u64) -> Option<Tables> {
    let mut block = vec![0u8; Rsdp::len()];
    let mut offsets = Vec::new();
    let mut add = |block: &mut Vec<u8>, table: Vec<u8>| -> u64 {
        let at = place(block, &table);
        offsets.push(([table[0], table[1], table[2], table[3]], at));
        base + at as u64
    };

    let dsdt_at = add(&mut block, dsdt::build());
    let fadt_at = add(&mut block, fadt(dsdt_at));
    let madt_at = add(&mut block, madt(vcpus));
    let mcfg_at = add(&mut block, mcfg());
    let mut xsdt = XSDT::new(OEM_ID, OEM_TABLE_ID, OEM_REVISION);
    for at in [fadt_at, madt_at, mcfg_at] {
        xsdt.add_entry(at);
    }
    let xsdt_at = add(&mut block, bytes_of(&xsdt));

    let rsdp = bytes_of(&Rsdp::new(OEM_ID, xsdt_at));
    block[..rsdp.len()].copy_from_slice(&rsdp);
    (block.len() as u64 <= ACPI_BYTES).then_some(Tables {
        bytes: block,
        offsets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arch::layout::ACPI_START;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn table<'a>(t: &'a Tables, signature: &[u8; 4]) -> &'a [u8] {
        let at = t.offsets.iter().find(|(s, _)| s == signature).unwrap().1;
        let length = u32::from_le_bytes(t.bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        &t.bytes[at..at + length]
    }

    fn sum(bytes: &[u8]) -> u8 {
        bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b))
    }

    #[test]
    fn the_root_pointer_leads_to_every_table_and_every_sum_is_zero() {
        let t = build(4, ACPI_START).unwrap();
        assert_eq!(&t.bytes[..8], b"RSD PTR ");
        assert_eq!(sum(&t.bytes[..20]), 0);
        assert_eq!(sum(&t.bytes[..36]), 0);
        let xsdt_at = u64::from_le_bytes(t.bytes[24..32].try_into().unwrap());
        let xsdt = table(&t, b"XSDT");
        assert_eq!(
            xsdt_at,
            ACPI_START + t.offsets.iter().find(|(s, _)| s == b"XSDT").unwrap().1 as u64
        );
        let mut found = Vec::new();
        for entry in xsdt[36..].as_chunks::<8>().0 {
            let at = usize::try_from(u64::from_le_bytes(*entry) - ACPI_START).unwrap();
            found.push(t.bytes[at..at + 4].to_vec());
        }
        assert_eq!(
            found,
            [b"FACP".to_vec(), b"APIC".to_vec(), b"MCFG".to_vec()]
        );
        for signature in [b"DSDT", b"FACP", b"APIC", b"MCFG", b"XSDT"] {
            assert_eq!(sum(table(&t, signature)), 0, "{signature:?}");
        }
    }

    #[test]
    fn the_fadt_is_hardware_reduced_with_reset_and_sleep_registers() {
        let t = build(1, ACPI_START).unwrap();
        let fadt = table(&t, b"FACP");
        assert_eq!(fadt.len(), 276);
        let flags = u32::from_le_bytes(fadt[112..116].try_into().unwrap());
        assert_ne!(flags & (1 << 20), 0);
        assert_ne!(flags & (1 << 10), 0);
        // The reset register: I/O port 0x64, value 0xFE.
        assert_eq!((fadt[116], fadt[120], fadt[128]), (1, 0x64, 0xFE));
        // The sleep control register: I/O port 0x600.
        assert_eq!((fadt[244], &fadt[248..250]), (1, &[0x00, 0x06][..]));
        let dsdt_at = u64::from_le_bytes(fadt[140..148].try_into().unwrap());
        let at = usize::try_from(dsdt_at - ACPI_START).unwrap();
        assert_eq!(&t.bytes[at..at + 4], b"DSDT");
    }

    #[test]
    fn the_madt_has_a_local_apic_for_each_processor_and_one_io_apic() {
        let t = build(4, ACPI_START).unwrap();
        let madt = table(&t, b"APIC");
        let lapic = u32::from_le_bytes(madt[36..40].try_into().unwrap());
        assert_eq!(u64::from(lapic), LAPIC_START);
        let mut at = 44;
        let mut kinds = Vec::new();
        while at < madt.len() {
            kinds.push((madt[at], madt[at + 3]));
            at += usize::from(madt[at + 1]);
        }
        // Four local APICs with ids 0 to 3, then the I/O APIC.
        assert_eq!(kinds[..4], [(0, 0), (0, 1), (0, 2), (0, 3)]);
        assert_eq!(kinds[4].0, 1);
        assert_eq!(madt[at - 12 + 2], 4);
    }

    #[test]
    fn the_mcfg_names_bus_zero() {
        let t = build(1, ACPI_START).unwrap();
        let mcfg = table(&t, b"MCFG");
        let base = u64::from_le_bytes(mcfg[44..52].try_into().unwrap());
        assert_eq!(base, ECAM_START);
        assert_eq!((mcfg[54], mcfg[55]), (0, 0));
        assert!(build(240, ACPI_START).is_some());
    }
}
