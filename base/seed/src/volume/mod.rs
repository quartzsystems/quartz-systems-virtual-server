// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The seed volume: a FAT file system, on a whole disk or a partition,
//! whose volume label is the appliance's seed label.

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{find, read};

/// Bytes of a boot sector.
pub const SECTOR: usize = 512;

/// The volume label in a FAT boot sector, trimmed; `None` when the
/// sector is not one.
pub fn fat_label(sector: &[u8]) -> Option<String> {
    if sector.len() < SECTOR || sector[510..512] != [0x55, 0xAA] {
        return None;
    }
    // The extended boot record is at 36 for FAT12 and FAT16, at 64 for
    // FAT32; each has its signature byte, the label and the type.
    let label = [(38, 43, 54, &b"FAT1"[..]), (66, 71, 82, &b"FAT32"[..])]
        .into_iter()
        .find(|(signature, _, kind, word)| {
            sector[*signature] == 0x29 && sector[*kind..].starts_with(word)
        })
        .map(|(_, label, _, _)| &sector[label..label + 11])?;
    Some(String::from_utf8_lossy(label).trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sector(at: usize, kind_at: usize, kind: &[u8], label: &[u8; 11]) -> Vec<u8> {
        let mut s = vec![0u8; SECTOR];
        s[510] = 0x55;
        s[511] = 0xAA;
        s[at - 5] = 0x29;
        s[at..at + 11].copy_from_slice(label);
        s[kind_at..kind_at + kind.len()].copy_from_slice(kind);
        s
    }

    #[test]
    fn the_label_is_read_from_either_kind_of_fat() {
        let fat16 = sector(43, 54, b"FAT16   ", b"QSDEMOSEED ");
        assert_eq!(fat_label(&fat16).as_deref(), Some("QSDEMOSEED"));
        let fat32 = sector(71, 82, b"FAT32   ", b"SEED       ");
        assert_eq!(fat_label(&fat32).as_deref(), Some("SEED"));
    }

    #[test]
    fn other_sectors_have_no_label() {
        assert_eq!(fat_label(&[0u8; SECTOR]), None);
        assert_eq!(fat_label(&[0u8; 100]), None);
        // The signature without a FAT's extended boot record: a GPT's
        // protective MBR, say.
        let mut mbr = vec![0u8; SECTOR];
        mbr[510] = 0x55;
        mbr[511] = 0xAA;
        assert_eq!(fat_label(&mbr), None);
        let mut no_sig = sector(43, 54, b"FAT16   ", b"QSDEMOSEED ");
        no_sig[511] = 0;
        assert_eq!(fat_label(&no_sig), None);
    }
}
