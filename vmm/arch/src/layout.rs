// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The guest's physical memory map (`docs/VMM.md`).
//!
//! | From | To | What |
//! |---|---|---|
//! | `0` | `9FC00` | RAM: the boot structures |
//! | `9FC00` | `A0000` | reserved |
//! | `E0000` | `F0000` | the ACPI tables |
//! | `F0000` | `FF000` | the SMBIOS tables |
//! | `FF000` | `100000` | the reset vector's page |
//! | `100000` | 3 GiB at most | RAM |
//! | `C0000000` | `E8000000` | PCI memory space |
//! | `E8000000` | `E8100000` | PCI configuration space (bus 0) |
//! | `FEC00000` | | the I/O APIC |
//! | `FEE00000` | | the local APICs |
//! | `100000000` | | RAM beyond 3 GiB |

use crate::Error;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

/// Where low memory ends and the reserved page before the VGA hole starts.
pub const LOW_RAM_END: u64 = 0x9_FC00;
/// The end of the first megabyte's hole.
pub const HIGH_RAM_START: u64 = 0x10_0000;
/// The ACPI tables: the root pointer first, where a scan finds it.
pub const ACPI_START: u64 = 0xE_0000;
/// Bytes kept for the ACPI tables: up to the SMBIOS tables.
pub const ACPI_BYTES: u64 = 0x1_0000;
/// The SMBIOS tables: the entry point first, where a scan finds it.
pub const SMBIOS_START: u64 = 0xF_0000;
/// Bytes kept for them: up to the page of the reset vector.
pub const SMBIOS_BYTES: u64 = 0xF000;
/// The last page of the first megabyte, where the reset vector is.
pub const RESET_PAGE_START: u64 = 0xF_F000;
/// Where a processor starts after a reset, and where a kernel that
/// finds no other way to restart jumps.
pub const RESET_VECTOR: u64 = 0xF_FFF0;

/// The GDT.
pub const GDT_START: u64 = 0x500;
/// The zero page.
pub const ZERO_PAGE_START: u64 = 0x7000;
/// The first page table; five more pages follow it.
pub const PML4_START: u64 = 0x9000;
/// The page directory pointer table.
pub const PDPT_START: u64 = 0xA000;
/// The first of four page directories.
pub const PD_START: u64 = 0xB000;
/// The boot stack's top. The kernel sets its own at once.
pub const BOOT_STACK: u64 = 0x8FF0;
/// The command line: empty, the kernel's own is baked in.
pub const CMDLINE_START: u64 = 0x2_0000;
/// Where the kernel is loaded.
pub const KERNEL_START: u64 = 0x100_0000;

/// Where RAM below 4 GiB stops and PCI memory space starts.
pub const PCI_MEM_START: u64 = 0xC000_0000;
/// The end of PCI memory space.
pub const PCI_MEM_END: u64 = 0xE800_0000;
/// PCI configuration space, memory mapped.
pub const ECAM_START: u64 = 0xE800_0000;
/// One bus: 32 devices of 8 functions of 4096 bytes.
pub const ECAM_BYTES: u64 = 0x10_0000;
/// The I/O APIC.
pub const IOAPIC_START: u64 = 0xFEC0_0000;
/// The local APICs.
pub const LAPIC_START: u64 = 0xFEE0_0000;
/// Three pages KVM wants for itself on Intel processors.
pub const TSS_START: u64 = 0xFFFB_D000;
/// One page more of the same kind.
pub const IDENTITY_MAP_START: u64 = 0xFFFB_C000;
/// Where RAM goes on beyond the 32-bit hole.
pub const HIGH_START: u64 = 0x1_0000_0000;

/// The least memory a partition may have.
pub const MIN_MEMORY: u64 = 128 * MIB;
/// The most.
pub const MAX_MEMORY: u64 = 1024 * GIB;

/// The serial port's I/O ports.
pub const SERIAL_PORT: u16 = 0x3F8;
/// The serial port's interrupt.
pub const SERIAL_IRQ: u32 = 4;
/// The CMOS clock's two ports.
pub const CMOS_PORT: u16 = 0x70;
/// The clock's interrupt, which is never raised.
pub const CMOS_IRQ: u32 = 8;
/// The keyboard controller's ports: only its reset line is there.
pub const I8042_PORT: u16 = 0x60;
/// The command port of the keyboard controller.
pub const I8042_COMMAND_PORT: u16 = 0x64;
/// The value that pulses the reset line.
pub const I8042_RESET: u8 = 0xFE;
/// The ACPI sleep control register; the status register follows it.
pub const SLEEP_PORT: u16 = 0x600;
/// The sleep type of S5, soft off.
pub const SLEEP_TYPE_S5: u8 = 5;
/// The PCI configuration address port; the data port is four on.
pub const PCI_CONFIG_PORT: u16 = 0xCF8;

/// Interrupts 0 to 23 are the I/O APIC's pins; message interrupts
/// are routed from here up.
pub const FIRST_MSI_GSI: u32 = 24;

/// What an E820 entry says of its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Usable memory.
    Ram,
    /// Not to be touched.
    Reserved,
    /// The ACPI tables.
    Acpi,
}

/// One range of the map handed to the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    /// First byte.
    pub start: u64,
    /// Length.
    pub bytes: u64,
    /// What it is.
    pub kind: Kind,
}

/// A partition's RAM, split around the 32-bit hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ram {
    /// Bytes from 0 up, 3 GiB at most.
    pub low: u64,
    /// Bytes from 4 GiB up.
    pub high: u64,
}

impl Ram {
    /// The split of `bytes` of memory: whole megabytes, within the limits.
    pub fn for_bytes(bytes: u64) -> Result<Self, Error> {
        if !bytes.is_multiple_of(MIB) {
            return Err(Error::Memory(bytes, "not whole megabytes"));
        }
        if bytes < MIN_MEMORY {
            return Err(Error::Memory(bytes, "less than 128 MiB"));
        }
        if bytes > MAX_MEMORY {
            return Err(Error::Memory(bytes, "more than 1024 GiB"));
        }
        let low = bytes.min(PCI_MEM_START);
        Ok(Self {
            low,
            high: bytes - low,
        })
    }

    /// The regions to map: start and length of each.
    pub fn regions(&self) -> Vec<(u64, u64)> {
        let mut regions = vec![(0, self.low)];
        if self.high > 0 {
            regions.push((HIGH_START, self.high));
        }
        regions
    }

    /// The map the kernel is given.
    pub fn e820(&self) -> Vec<Range> {
        let mut map = vec![
            Range {
                start: 0,
                bytes: LOW_RAM_END,
                kind: Kind::Ram,
            },
            Range {
                start: LOW_RAM_END,
                bytes: 0xA_0000 - LOW_RAM_END,
                kind: Kind::Reserved,
            },
            Range {
                start: ACPI_START,
                bytes: ACPI_BYTES,
                kind: Kind::Acpi,
            },
            Range {
                start: SMBIOS_START,
                bytes: SMBIOS_BYTES,
                kind: Kind::Reserved,
            },
            Range {
                start: RESET_PAGE_START,
                bytes: HIGH_RAM_START - RESET_PAGE_START,
                kind: Kind::Reserved,
            },
            Range {
                start: HIGH_RAM_START,
                bytes: self.low - HIGH_RAM_START,
                kind: Kind::Ram,
            },
            Range {
                start: ECAM_START,
                bytes: ECAM_BYTES,
                kind: Kind::Reserved,
            },
        ];
        if self.high > 0 {
            map.push(Range {
                start: HIGH_START,
                bytes: self.high,
                kind: Kind::Ram,
            });
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_memory_is_one_region_below_the_hole() {
        let ram = Ram::for_bytes(512 * MIB).unwrap();
        assert_eq!(ram.regions(), [(0, 512 * MIB)]);
        let map = ram.e820();
        assert_eq!(map.len(), 7);
        assert_eq!(map[5].start + map[5].bytes, 512 * MIB);
    }

    #[test]
    fn large_memory_goes_on_beyond_four_gigabytes() {
        let ram = Ram::for_bytes(8 * GIB).unwrap();
        assert_eq!(ram.regions(), [(0, 3 * GIB), (HIGH_START, 5 * GIB)]);
        let last = *ram.e820().last().unwrap();
        assert_eq!(
            (last.start, last.bytes, last.kind),
            (HIGH_START, 5 * GIB, Kind::Ram)
        );
    }

    #[test]
    fn the_map_does_not_overlap_and_is_in_order() {
        for bytes in [128 * MIB, 3 * GIB, 3 * GIB + MIB, 64 * GIB] {
            let map = Ram::for_bytes(bytes).unwrap().e820();
            for pair in map.windows(2) {
                assert!(pair[0].start + pair[0].bytes <= pair[1].start, "{bytes}");
            }
            let ram: u64 = map
                .iter()
                .filter(|r| r.kind == Kind::Ram)
                .map(|r| r.bytes)
                .sum();
            // The first megabyte's holes are not usable.
            assert_eq!(ram, bytes - (HIGH_RAM_START - LOW_RAM_END));
        }
    }

    #[test]
    fn memory_out_of_range_is_refused() {
        assert!(Ram::for_bytes(64 * MIB).is_err());
        assert!(Ram::for_bytes(128 * MIB + 5).is_err());
        assert!(Ram::for_bytes(2048 * GIB).is_err());
    }
}
