// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's SMBIOS tables: who made the machine, what it is, and
//! its serial number. A partition has no firmware to give them, so the
//! VMM writes them where a kernel without firmware looks: a 64-bit
//! entry point (`_SM3_`) at `F0000`, the structures after it.
//!
//! | Type | What | Says |
//! |---|---|---|
//! | 0 | BIOS | the vendor and the release of QS/VS; a virtual machine |
//! | 1 | System | manufacturer, product, the release, the serial number, a UUID made from it |
//! | 3 | Chassis | the manufacturer and the serial number again |
//! | 127 | End | |
//!
//! The manufacturer and the product are what QS/400 knows a partition
//! of QS/VS by (`docs/QS400_GUEST.md`).

use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

use crate::Error;
use crate::layout::{SMBIOS_BYTES, SMBIOS_START};

/// The manufacturer of a partition.
pub const MANUFACTURER: &str = "Quartz Systems";
/// The product: what a partition is.
pub const PRODUCT: &str = "QS/VS partition";
/// The family.
pub const FAMILY: &str = "QS/VS";
/// The entry point's length.
const ENTRY_BYTES: usize = 24;
/// The longest string a structure is given.
const STRING_MAX: usize = 64;

/// What the tables say of one partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity<'a> {
    /// The system serial number.
    pub serial: &'a str,
    /// The release of the host, `V1R0M0`.
    pub release: &'a str,
}

/// A string as a structure holds it: printable ASCII, bounded.
fn clean(text: &str) -> Vec<u8> {
    let bytes: Vec<u8> = text
        .bytes()
        .filter(|b| b.is_ascii_graphic() || *b == b' ')
        .take(STRING_MAX)
        .collect();
    if bytes.is_empty() {
        b" ".to_vec()
    } else {
        bytes
    }
}

/// One structure: the formatted area (its type, length and handle
/// first) and the strings it numbers from 1.
fn structure(kind: u8, handle: u16, body: &[u8], strings: &[&str]) -> Vec<u8> {
    #[allow(clippy::cast_possible_truncation)] // a formatted area is a few bytes
    let mut out = vec![kind, (body.len() + 4) as u8];
    out.extend_from_slice(&handle.to_le_bytes());
    out.extend_from_slice(body);
    for text in strings {
        out.extend_from_slice(&clean(text));
        out.push(0);
    }
    if strings.is_empty() {
        out.push(0);
    }
    out.push(0);
    out
}

/// A UUID for the serial number: the same serial, the same UUID.
pub fn uuid(serial: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (half, seed) in [
        (0usize, 0xcbf2_9ce4_8422_2325u64),
        (8, 0x8422_2325_cbf2_9ce4),
    ] {
        let mut hash = seed;
        for byte in FAMILY.bytes().chain(serial.bytes()) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        out[half..half + 8].copy_from_slice(&hash.to_le_bytes());
    }
    // A random-form UUID, by its version and variant.
    out[7] = (out[7] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

/// The structure table.
fn table(identity: Identity<'_>) -> Vec<u8> {
    let version = format!("{FAMILY} {}", identity.release);
    let mut out = Vec::new();

    // Type 0. Characteristics are not given (bit 3); the second
    // extension byte says a virtual machine (bit 4).
    let mut bios = vec![1, 2];
    bios.extend_from_slice(&0xE000u16.to_le_bytes());
    bios.extend_from_slice(&[3, 0]);
    bios.extend_from_slice(&(1u64 << 3).to_le_bytes());
    bios.extend_from_slice(&[0, 0x10, 1, 0, 0xFF, 0xFF]);
    out.extend(structure(
        0,
        0x0000,
        &bios,
        &[MANUFACTURER, &version, "01/01/2026"],
    ));

    // Type 1. Woken by the power switch (6).
    let mut system = vec![1, 2, 3, 4];
    system.extend_from_slice(&uuid(identity.serial));
    system.extend_from_slice(&[6, 5, 6]);
    out.extend(structure(
        1,
        0x0100,
        &system,
        &[
            MANUFACTURER,
            PRODUCT,
            identity.release,
            identity.serial,
            "PARTITION",
            FAMILY,
        ],
    ));

    // Type 3. Of no physical kind (1, other); its states are safe (3),
    // its security unknown (2).
    let mut chassis = vec![1, 1, 2, 3, 0, 3, 3, 3, 2];
    chassis.extend_from_slice(&[0; 4]);
    chassis.extend_from_slice(&[0, 0, 0, 0, 0]);
    out.extend(structure(
        3,
        0x0300,
        &chassis,
        &[MANUFACTURER, identity.release, identity.serial],
    ));

    out.extend(structure(127, 0x7F00, &[], &[]));
    out
}

/// The entry point and the table, as they are written at `base`.
pub fn build(identity: Identity<'_>, base: u64) -> Vec<u8> {
    let table = table(identity);
    let mut entry = Vec::with_capacity(ENTRY_BYTES + table.len());
    entry.extend_from_slice(b"_SM3_");
    // The checksum, then: the entry's length, version 3.0, document
    // revision 0, entry point revision 1, reserved.
    #[allow(clippy::cast_possible_truncation)] // 24
    entry.extend_from_slice(&[0, ENTRY_BYTES as u8, 3, 0, 0, 1, 0]);
    #[allow(clippy::cast_possible_truncation)] // a few hundred bytes
    entry.extend_from_slice(&(table.len() as u32).to_le_bytes());
    entry.extend_from_slice(&(base + ENTRY_BYTES as u64).to_le_bytes());
    let sum = entry.iter().fold(0u8, |sum, b| sum.wrapping_add(*b));
    entry[5] = 0u8.wrapping_sub(sum);
    entry.extend_from_slice(&table);
    entry
}

/// Write the tables into guest memory.
pub fn write(mem: &GuestMemoryMmap, identity: Identity<'_>) -> Result<(), Error> {
    let bytes = build(identity, SMBIOS_START);
    if bytes.len() as u64 > SMBIOS_BYTES {
        return Err(Error::GuestMemory("the SMBIOS tables"));
    }
    mem.write_slice(&bytes, GuestAddress(SMBIOS_START))
        .map_err(|_| Error::GuestMemory("the SMBIOS tables"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Kind, Ram};

    const IDENTITY: Identity<'static> = Identity {
        serial: "K2M4103",
        release: "V1R0M0",
    };

    /// The structures of a table: type, formatted area, strings.
    fn structures(mut table: &[u8]) -> Vec<(u8, Vec<u8>, Vec<String>)> {
        let mut out = Vec::new();
        while table.len() >= 4 {
            let (kind, len) = (table[0], table[1] as usize);
            let body = table[4..len].to_vec();
            let mut strings = Vec::new();
            let mut at = len;
            loop {
                let end = at + table[at..].iter().position(|b| *b == 0).unwrap();
                if end == at {
                    // An area with no strings ends in two zeros.
                    at += if strings.is_empty() { 2 } else { 1 };
                    break;
                }
                strings.push(String::from_utf8(table[at..end].to_vec()).unwrap());
                at = end + 1;
            }
            out.push((kind, body, strings));
            table = &table[at..];
        }
        out
    }

    #[test]
    fn the_entry_point_is_found_by_a_scan_and_sums_to_zero() {
        let bytes = build(IDENTITY, SMBIOS_START);
        assert_eq!(&bytes[..5], b"_SM3_");
        assert_eq!(bytes[6], 24);
        assert_eq!(bytes[..24].iter().fold(0u8, |s, b| s.wrapping_add(*b)), 0);
        assert_eq!((bytes[7], bytes[8]), (3, 0));
        let size = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(size, bytes.len() - 24);
        let at = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        assert_eq!(at, SMBIOS_START + 24);
        // A kernel without firmware scans F0000 to FFFFF in steps of 16.
        assert_eq!(SMBIOS_START % 16, 0);
        assert!((0xF_0000..0x10_0000).contains(&SMBIOS_START));
    }

    #[test]
    fn the_system_is_a_partition_of_qs_vs_with_its_serial_number() {
        let bytes = build(IDENTITY, SMBIOS_START);
        let all = structures(&bytes[24..]);
        let kinds: Vec<u8> = all.iter().map(|s| s.0).collect();
        assert_eq!(kinds, [0, 1, 3, 127]);
        let (_, bios, strings) = &all[0];
        assert_eq!(bios.len() + 4, 0x18);
        assert_eq!(strings[0], "Quartz Systems");
        assert_eq!(strings[1], "QS/VS V1R0M0");
        assert_eq!(bios[15] & 0x10, 0x10);
        let (_, system, strings) = &all[1];
        assert_eq!(system.len() + 4, 0x1B);
        assert_eq!(strings[usize::from(system[0]) - 1], "Quartz Systems");
        assert_eq!(strings[usize::from(system[1]) - 1], "QS/VS partition");
        assert_eq!(strings[usize::from(system[3]) - 1], "K2M4103");
        assert_eq!(system[4..20], uuid("K2M4103"));
        let (_, chassis, strings) = &all[2];
        assert_eq!(chassis.len() + 4, 0x16);
        assert_eq!(strings[usize::from(chassis[3]) - 1], "K2M4103");
        assert!(all[3].1.is_empty());
        // As QS/400 tells a partition of QS/VS: by the product's name.
        assert!(PRODUCT.to_ascii_lowercase().contains("qs/vs"));
        assert!(MANUFACTURER.to_ascii_lowercase().contains("quartz"));
    }

    #[test]
    fn a_serial_number_has_its_own_uuid_and_odd_strings_are_made_plain() {
        assert_eq!(uuid("K2M4103"), uuid("K2M4103"));
        assert_ne!(uuid("K2M4103"), uuid("K2M4104"));
        assert_eq!(uuid("A")[7] >> 4, 4);
        assert_eq!(uuid("A")[8] >> 6, 2);
        let odd = Identity {
            serial: "A\0B\nC",
            release: "",
        };
        let bytes = build(odd, SMBIOS_START);
        let all = structures(&bytes[24..]);
        assert_eq!(all[1].2[3], "ABC");
        assert_eq!(all[1].2[2], " ");
        let long = "X".repeat(500);
        let bytes = build(
            Identity {
                serial: &long,
                release: &long,
            },
            SMBIOS_START,
        );
        assert!(bytes.len() as u64 <= SMBIOS_BYTES);
    }

    #[test]
    fn the_tables_are_written_where_the_kernel_is_told_not_to_use() {
        let ram = Ram::for_bytes(128 << 20).unwrap();
        let mem = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 128 << 20)]).unwrap();
        write(&mem, IDENTITY).unwrap();
        let mut anchor = [0u8; 5];
        mem.read_slice(&mut anchor, GuestAddress(SMBIOS_START))
            .unwrap();
        assert_eq!(&anchor, b"_SM3_");
        let range = ram
            .e820()
            .into_iter()
            .find(|r| r.start <= SMBIOS_START && SMBIOS_START < r.start + r.bytes)
            .unwrap();
        assert_eq!(range.kind, Kind::Reserved);
        assert_eq!((range.start, range.bytes), (SMBIOS_START, SMBIOS_BYTES));
    }
}
