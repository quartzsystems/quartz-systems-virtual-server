// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Reading a GPT: each partition's type, its own GUID and where it is.
//! 512-byte sectors, as every disk this host writes has.

use std::io::{self, Read, Seek, SeekFrom};

use uuid::Uuid;

const SECTOR: u64 = 512;

/// One partition of a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The GPT entry, from 1.
    pub index: u32,
    /// The partition's type.
    pub type_guid: Uuid,
    /// The partition's own GUID.
    pub guid: Uuid,
    /// Its first sector.
    pub first_lba: u64,
    /// Its last sector.
    pub last_lba: u64,
}

impl Entry {
    /// Where it starts, in bytes.
    pub fn offset(&self) -> u64 {
        self.first_lba * SECTOR
    }

    /// Its length in bytes.
    pub fn bytes(&self) -> u64 {
        (self.last_lba + 1 - self.first_lba) * SECTOR
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default())
}

fn guid_at(bytes: &[u8], at: usize) -> Uuid {
    Uuid::from_bytes_le(bytes[at..at + 16].try_into().unwrap_or_default())
}

/// The partitions of the table on `dev`; none when it has no GPT.
pub fn entries<D: Read + Seek>(dev: &mut D) -> io::Result<Vec<Entry>> {
    let mut header = [0u8; 92];
    dev.seek(SeekFrom::Start(SECTOR))?;
    dev.read_exact(&mut header)?;
    if &header[..8] != b"EFI PART" {
        return Ok(Vec::new());
    }
    let count = u32_at(&header, 80).min(1024);
    let size = u32_at(&header, 84).clamp(128, 4096) as usize;
    dev.seek(SeekFrom::Start(u64_at(&header, 72) * SECTOR))?;
    let mut entry = vec![0u8; size];
    let mut out = Vec::new();
    for index in 1..=count {
        dev.read_exact(&mut entry)?;
        let type_guid = guid_at(&entry, 0);
        if !type_guid.is_nil() {
            out.push(Entry {
                index,
                type_guid,
                guid: guid_at(&entry, 16),
                first_lba: u64_at(&entry, 32),
                last_lba: u64_at(&entry, 40),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout;
    use appliance::LoadSourceNames;
    use diskimg::layout::{ESP_TYPE_GUID, Layout};
    use std::io::Cursor;

    #[test]
    fn a_written_table_reads_back() {
        let names = &LoadSourceNames::DEMO;
        let bytes = diskimg::layout::min_bytes(&layout::install_parts(names));
        let table = Layout::for_bytes(&layout::install_parts(names), bytes).unwrap();
        let guids = layout::fixed_guids(names);
        let esp = vec![0u8; usize::try_from(layout::ESP_BYTES).unwrap()];
        let mut disk = Cursor::new(vec![0u8; usize::try_from(bytes).unwrap()]);
        diskimg::write_layout(&mut disk, &table, &guids, &[(1, &esp)], &mut |_, _| {}).unwrap();
        let found = entries(&mut disk).unwrap();
        let types: Vec<Uuid> = found.iter().map(|e| e.type_guid).collect();
        assert_eq!(
            types,
            [
                ESP_TYPE_GUID,
                layout::record_type(names),
                layout::data_type(names)
            ]
        );
        assert_eq!(found[2].guid, guids.parts[2]);
        assert_eq!(found[0].offset(), table.parts[0].part.offset());
        assert_eq!(found[1].bytes(), layout::RECORD_BYTES);
        // A disk with no table has no partitions.
        assert!(
            entries(&mut Cursor::new(vec![0u8; 4096]))
                .unwrap()
                .is_empty()
        );
    }
}
