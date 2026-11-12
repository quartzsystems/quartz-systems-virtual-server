// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The SMBIOS structure table, as the kernel gives it raw in
//! `/sys/firmware/dmi/tables/DMI`.
//!
//! A structure is a formatted area, whose second byte is its length,
//! and then a string set: strings each ended by a zero byte, the set
//! ended by one more. A field that names a string holds its number,
//! counted from 1.
//!
//! - [`structures`]: the walk.
//! - [`system`], [`slots`], [`oem_strings`]: types 1, 9 and 11.

mod records;

use std::fs;
use std::path::Path;

pub use records::{SlotRecord, SystemRecord, oem_strings, slots, system};

/// The type that ends the table.
const END_OF_TABLE: u8 = 127;

/// One structure of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Structure<'a> {
    /// The structure type.
    pub kind: u8,
    /// The handle.
    pub handle: u16,
    /// The formatted area, header included, so offsets are the
    /// specification's.
    pub formatted: &'a [u8],
    /// The string set, in order.
    pub strings: Vec<&'a [u8]>,
}

impl Structure<'_> {
    /// The byte at `offset` of the formatted area.
    pub fn byte(&self, offset: usize) -> Option<u8> {
        self.formatted.get(offset).copied()
    }

    /// The little-endian word at `offset` of the formatted area.
    pub fn word(&self, offset: usize) -> Option<u16> {
        Some(u16::from_le_bytes([
            self.byte(offset)?,
            self.byte(offset + 1)?,
        ]))
    }

    /// String number `index`, trimmed; `None` for 0 or a number past the
    /// set.
    pub fn string(&self, index: u8) -> Option<String> {
        let bytes = self.strings.get(usize::from(index).checked_sub(1)?)?;
        Some(String::from_utf8_lossy(bytes).trim().to_string())
    }

    /// The string the byte at `offset` names; empty when there is none.
    pub fn string_at(&self, offset: usize) -> String {
        self.byte(offset)
            .and_then(|index| self.string(index))
            .unwrap_or_default()
    }
}

/// The raw table under `sys` (`/sys`); empty when it is unreadable.
pub fn table(sys: &Path) -> Vec<u8> {
    fs::read(sys.join("firmware/dmi/tables/DMI")).unwrap_or_default()
}

/// The structures of a table, in order. The walk stops at the
/// end-of-table structure or at the first structure that does not fit.
pub fn structures(table: &[u8]) -> Vec<Structure<'_>> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(header) = table.get(at..at + 4) {
        let kind = header[0];
        let length = usize::from(header[1]);
        if kind == END_OF_TABLE || length < 4 {
            break;
        }
        let Some(formatted) = table.get(at..at + length) else {
            break;
        };
        let mut strings = Vec::new();
        let mut pos = at + length;
        loop {
            let rest = table.get(pos..).unwrap_or_default();
            let Some(end) = rest.iter().position(|b| *b == 0) else {
                // The set runs off the table.
                pos = table.len();
                break;
            };
            if end == 0 {
                // An empty set is two zero bytes.
                pos += if strings.is_empty() { 2 } else { 1 };
                break;
            }
            strings.push(&rest[..end]);
            pos += end + 1;
        }
        found.push(Structure {
            kind,
            handle: u16::from_le_bytes([header[2], header[3]]),
            formatted,
            strings,
        });
        at = pos;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::structure;

    #[test]
    fn the_walk_finds_each_structure_and_its_strings() {
        let mut table = structure(0, &[1, 2, 0, 0], &["Vendor", "1.0"]);
        table.extend(structure(32, &[0; 7], &[]));
        table.extend(structure(11, &[1], &["only"]));
        table.extend(structure(127, &[], &[]));
        table.extend(structure(11, &[1], &["after the end"]));
        let found = structures(&table);
        let kinds: Vec<u8> = found.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [0, 32, 11]);
        assert_eq!(found[0].string(1).as_deref(), Some("Vendor"));
        assert_eq!(found[0].string(2).as_deref(), Some("1.0"));
        assert_eq!(found[0].string(0), None);
        assert_eq!(found[0].string(3), None);
        assert_eq!(found[0].string_at(5), "1.0");
        assert_eq!(found[0].string_at(6), "");
        assert_eq!(found[0].string_at(99), "");
        assert!(found[1].strings.is_empty());
        assert_eq!(found[2].string(1).as_deref(), Some("only"));
    }

    #[test]
    fn words_are_little_endian_and_bounded() {
        let table = structure(9, &[0x34, 0x12], &[]);
        let found = structures(&table);
        assert_eq!(found[0].handle, 9);
        assert_eq!(found[0].word(4), Some(0x1234));
        assert_eq!(found[0].word(5), None);
        assert_eq!(found[0].byte(6), None);
    }

    #[test]
    fn a_damaged_table_stops_the_walk_without_panicking() {
        assert!(structures(&[]).is_empty());
        assert!(structures(&[1, 2]).is_empty());
        // A length shorter than the header.
        assert!(structures(&[1, 3, 0, 0, 0, 0]).is_empty());
        // A length past the end.
        assert!(structures(&[1, 200, 0, 0, 0, 0]).is_empty());
        // A string set that is never ended.
        let found = structures(&[11, 5, 0, 0, 1, b'a', b'b']);
        assert_eq!(found.len(), 1);
        assert!(found[0].strings.is_empty());
        // A structure cut off after a good one.
        let mut table = structure(11, &[1], &["good"]);
        table.extend([1, 27, 0]);
        assert_eq!(structures(&table).len(), 1);
    }

    #[test]
    fn an_unreadable_table_is_empty() {
        assert!(table(Path::new("/nonexistent/hostinv/sys")).is_empty());
    }
}
