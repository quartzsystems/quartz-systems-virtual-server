// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The page tables and the GDT the boot processor starts with.

use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

use crate::Error;
use crate::layout::{GDT_START, PD_START, PDPT_START, PML4_START};

/// Present, writable.
const PAGE_TABLE: u64 = 0x03;
/// Present, writable, a 2 MiB page.
const PAGE_2M: u64 = 0x83;
/// Gigabytes mapped one to one.
const MAPPED_GIB: u64 = 4;

/// The selector of the 64-bit code segment.
pub const CODE_SELECTOR: u16 = 0x10;
/// The selector of the data segment.
pub const DATA_SELECTOR: u16 = 0x18;
/// The selector of the task state segment.
pub const TSS_SELECTOR: u16 = 0x20;

/// One GDT entry from its flags (the access byte and the high nibble
/// of byte 6), base and limit.
const fn gdt_entry(flags: u16, base: u32, limit: u32) -> u64 {
    ((base as u64 & 0xFF00_0000) << 32)
        | ((flags as u64 & 0xF0FF) << 40)
        | ((limit as u64 & 0x000F_0000) << 32)
        | ((base as u64 & 0x00FF_FFFF) << 16)
        | (limit as u64 & 0xFFFF)
}

/// The GDT: null, null, 64-bit code, data, the TSS.
pub const GDT: [u64; 5] = [
    0,
    0,
    gdt_entry(0xA09B, 0, 0xF_FFFF),
    gdt_entry(0xC093, 0, 0xF_FFFF),
    gdt_entry(0x808B, 0, 0xF_FFFF),
];

/// Bytes of the GDT, less one: its limit.
pub const GDT_LIMIT: u16 = 5 * 8 - 1;

/// Write the GDT into guest memory.
pub fn write_gdt(mem: &GuestMemoryMmap) -> Result<(), Error> {
    for (i, entry) in (0u64..).zip(GDT) {
        mem.write_obj(entry, GuestAddress(GDT_START + i * 8))
            .map_err(|_| Error::GuestMemory("the GDT"))?;
    }
    Ok(())
}

/// Write the page tables: the first four gigabytes one to one, in
/// 2 MiB pages.
pub fn write_page_tables(mem: &GuestMemoryMmap) -> Result<(), Error> {
    let put = |value: u64, at: u64| {
        mem.write_obj(value, GuestAddress(at))
            .map_err(|_| Error::GuestMemory("the page tables"))
    };
    put(PDPT_START | PAGE_TABLE, PML4_START)?;
    for gib in 0..MAPPED_GIB {
        let pd = PD_START + gib * 0x1000;
        put(pd | PAGE_TABLE, PDPT_START + gib * 8)?;
        for i in 0..512 {
            put((gib << 30) | (i << 21) | PAGE_2M, pd + i * 8)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_code_segment_is_long_mode_and_the_data_segment_flat() {
        // Access 0x9B: present, ring 0, code, readable, accessed. Flags
        // 0xA: 4 KiB granularity, long mode.
        assert_eq!(GDT[2], 0x00AF_9B00_0000_FFFF);
        // Access 0x93: present, ring 0, data, writable. Flags 0xC:
        // granularity, 32-bit.
        assert_eq!(GDT[3], 0x00CF_9300_0000_FFFF);
        assert_eq!(usize::from(CODE_SELECTOR), 2 * 8);
        assert_eq!(usize::from(DATA_SELECTOR), 3 * 8);
    }

    #[test]
    fn four_gigabytes_are_mapped_one_to_one() {
        let mem = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(0), 0x2_0000)]).unwrap();
        write_page_tables(&mem).unwrap();
        let get = |at: u64| mem.read_obj::<u64>(GuestAddress(at)).unwrap();
        assert_eq!(get(PML4_START), PDPT_START | 3);
        assert_eq!(get(PDPT_START + 3 * 8), (PD_START + 0x3000) | 3);
        // The 2 MiB page at 3 GiB + 4 MiB.
        assert_eq!(get(PD_START + 0x3000 + 2 * 8), 0xC040_0000 | 0x83);
        assert_eq!(get(PD_START + 8), 0x20_0000 | 0x83);
    }

    #[test]
    fn memory_too_small_for_the_tables_is_an_error() {
        let mem = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(0), 0x1000)]).unwrap();
        assert!(write_page_tables(&mem).is_err());
    }
}
