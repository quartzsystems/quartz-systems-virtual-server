// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition table of a load-source disk, read as a disk a guest
//! wrote: every count, size and address in it is checked before it is
//! used, both checksums must hold, and a partition that is not wholly
//! on the disk is not a partition. 512-byte sectors, as QS/400 writes.

use std::io::{Read, Seek, SeekFrom};

use uuid::Uuid;

/// Bytes of a sector.
pub const SECTOR: u64 = 512;
/// The most entries a table may have.
pub const MAX_ENTRIES: u32 = 256;
const HEADER_MIN: usize = 92;
const SECTOR_BYTES: usize = 512;

/// Why a disk has no usable table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GptError {
    /// The disk could not be read.
    #[error("the disk could not be read")]
    Io,
    /// No GPT header.
    #[error("no GPT header")]
    NoHeader,
    /// A field of the header is not one a table can have.
    #[error("the GPT header's {0} is not valid")]
    Header(&'static str),
    /// A checksum does not hold.
    #[error("the GPT {0} checksum does not hold")]
    Checksum(&'static str),
}

/// One partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition {
    /// The entry's number, from 1.
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

impl Partition {
    /// Where it starts, in bytes.
    pub fn offset(&self) -> u64 {
        self.first_lba * SECTOR
    }

    /// Its length in bytes.
    pub fn bytes(&self) -> u64 {
        (self.last_lba - self.first_lba + 1) * SECTOR
    }
}

/// CRC-32 as the GPT has it.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (!(crc & 1)).wrapping_add(1));
        }
    }
    !crc
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    (u64::from(u32_at(b, at + 4)) << 32) | u64::from(u32_at(b, at))
}

fn guid_at(b: &[u8], at: usize) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&b[at..at + 16]);
    Uuid::from_bytes_le(bytes)
}

/// The partitions of the table on `dev`, a disk of `disk_bytes`.
pub fn read<D: Read + Seek>(dev: &mut D, disk_bytes: u64) -> Result<Vec<Partition>, GptError> {
    let sectors = disk_bytes / SECTOR;
    let mut sector = [0u8; SECTOR_BYTES];
    dev.seek(SeekFrom::Start(SECTOR))
        .map_err(|_| GptError::Io)?;
    dev.read_exact(&mut sector).map_err(|_| GptError::Io)?;
    if &sector[..8] != b"EFI PART" {
        return Err(GptError::NoHeader);
    }
    let header_bytes = u32_at(&sector, 12) as usize;
    if !(HEADER_MIN..=sector.len()).contains(&header_bytes) {
        return Err(GptError::Header("size"));
    }
    let stored = u32_at(&sector, 16);
    let mut header = sector[..header_bytes].to_vec();
    header[16..20].fill(0);
    if crc32(&header) != stored {
        return Err(GptError::Checksum("header"));
    }
    if u64_at(&sector, 24) != 1 {
        return Err(GptError::Header("own address"));
    }
    let entries_lba = u64_at(&sector, 72);
    let count = u32_at(&sector, 80);
    let entry_bytes = u32_at(&sector, 84);
    if count == 0 || count > MAX_ENTRIES {
        return Err(GptError::Header("entry count"));
    }
    if !(128..=512).contains(&entry_bytes) || !entry_bytes.is_power_of_two() {
        return Err(GptError::Header("entry size"));
    }
    // 256 entries of 512 bytes at most: 128 KiB.
    let table_bytes = u64::from(count) * u64::from(entry_bytes);
    let table_end = entries_lba
        .checked_mul(SECTOR)
        .and_then(|start| start.checked_add(table_bytes));
    if entries_lba < 2 || table_end.is_none_or(|end| end > disk_bytes) {
        return Err(GptError::Header("entry table address"));
    }
    let mut table =
        vec![0u8; usize::try_from(table_bytes).map_err(|_| GptError::Header("entry count"))?];
    dev.seek(SeekFrom::Start(entries_lba * SECTOR))
        .map_err(|_| GptError::Io)?;
    dev.read_exact(&mut table).map_err(|_| GptError::Io)?;
    if crc32(&table) != u32_at(&sector, 88) {
        return Err(GptError::Checksum("entry table"));
    }
    let mut out = Vec::new();
    for (index, entry) in (1..).zip(table.chunks_exact(entry_bytes as usize)) {
        let type_guid = guid_at(entry, 0);
        let (first_lba, last_lba) = (u64_at(entry, 32), u64_at(entry, 40));
        // An entry that is not wholly on the disk is skipped, not trusted.
        if type_guid.is_nil() || first_lba < 2 || first_lba > last_lba || last_lba >= sectors {
            continue;
        }
        out.push(Partition {
            index,
            type_guid,
            guid: guid_at(entry, 16),
            first_lba,
            last_lba,
        });
    }
    Ok(out)
}

/// Make both checksums of the table in `disk` hold, where its header
/// names a table that is in `disk`. For the fuzzer, to reach what is
/// behind the checksums.
pub fn seal(disk: &mut [u8]) {
    let Some(header) = disk.get(SECTOR_BYTES..2 * SECTOR_BYTES) else {
        return;
    };
    let header_bytes = (u32_at(header, 12) as usize).clamp(HEADER_MIN, SECTOR_BYTES);
    let table_bytes =
        u32_at(header, 80).min(MAX_ENTRIES) as usize * u32_at(header, 84).min(512) as usize;
    let table = usize::try_from(u64_at(header, 72))
        .ok()
        .and_then(|lba| lba.checked_mul(SECTOR_BYTES))
        .and_then(|start| disk.get(start..start.checked_add(table_bytes)?))
        .map(crc32);
    let header = &mut disk[SECTOR_BYTES..2 * SECTOR_BYTES];
    if let Some(table) = table {
        header[88..92].copy_from_slice(&table.to_le_bytes());
    }
    header[16..20].fill(0);
    let sum = crc32(&header[..header_bytes]);
    header[16..20].copy_from_slice(&sum.to_le_bytes());
}

/// Build a disk's first sectors with a table of `parts` (type, first
/// and last sector), for the tests and the fuzzer's corpus.
pub fn build(disk_sectors: u64, parts: &[(Uuid, u64, u64)]) -> Vec<u8> {
    let mut table = vec![0u8; 128 * 128];
    for (i, (ty, first, last)) in parts.iter().enumerate() {
        let e = &mut table[i * 128..(i + 1) * 128];
        e[..16].copy_from_slice(&ty.to_bytes_le());
        e[16] = u8::try_from(i + 1).unwrap_or(0xFF);
        e[32..40].copy_from_slice(&first.to_le_bytes());
        e[40..48].copy_from_slice(&last.to_le_bytes());
    }
    let mut header = vec![0u8; 92];
    header[..8].copy_from_slice(b"EFI PART");
    header[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
    header[12..16].copy_from_slice(&92u32.to_le_bytes());
    header[24..32].copy_from_slice(&1u64.to_le_bytes());
    header[32..40].copy_from_slice(&(disk_sectors - 1).to_le_bytes());
    header[72..80].copy_from_slice(&2u64.to_le_bytes());
    header[80..84].copy_from_slice(&128u32.to_le_bytes());
    header[84..88].copy_from_slice(&128u32.to_le_bytes());
    header[88..92].copy_from_slice(&crc32(&table).to_le_bytes());
    let sum = crc32(&header);
    header[16..20].copy_from_slice(&sum.to_le_bytes());
    let mut disk = vec![0u8; 34 * SECTOR_BYTES];
    disk[512..512 + 92].copy_from_slice(&header);
    disk[1024..1024 + table.len()].copy_from_slice(&table);
    disk
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use uuid::uuid;

    const ESP: Uuid = uuid!("c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
    const OTHER: Uuid = uuid!("a0b7c2f4-5e13-4d9a-9f31-515334304c49");
    const SECTORS: u64 = 4096;

    fn disk() -> Vec<u8> {
        let mut d = build(SECTORS, &[(ESP, 64, 1023), (OTHER, 1024, 2047)]);
        d.resize(usize::try_from(SECTORS * SECTOR).unwrap(), 0);
        d
    }

    fn reseal(d: &mut [u8]) {
        seal(d);
    }

    fn parse(d: &[u8]) -> Result<Vec<Partition>, GptError> {
        read(&mut Cursor::new(d), d.len() as u64)
    }

    #[test]
    fn a_written_table_reads_back() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let parts = parse(&disk()).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!((parts[0].index, parts[0].type_guid), (1, ESP));
        assert_eq!((parts[0].offset(), parts[0].bytes()), (64 * 512, 960 * 512));
        assert_eq!(parts[1].first_lba, 1024);
    }

    #[test]
    fn a_damaged_table_is_refused_by_name() {
        assert_eq!(parse(&vec![0u8; 1 << 20]), Err(GptError::NoHeader));
        assert_eq!(parse(&[0u8; 700]), Err(GptError::Io));
        let mut d = disk();
        d[512 + 40] ^= 1;
        assert_eq!(parse(&d), Err(GptError::Checksum("header")));
        let mut d = disk();
        d[1024 + 33] ^= 1;
        assert_eq!(parse(&d), Err(GptError::Checksum("entry table")));
    }

    #[test]
    fn a_table_that_lies_with_good_checksums_is_still_checked() {
        // An entry count no table has.
        let mut d = disk();
        d[512 + 80..512 + 84].copy_from_slice(&u32::MAX.to_le_bytes());
        reseal(&mut d);
        assert_eq!(parse(&d), Err(GptError::Header("entry count")));
        // An entry size of zero, and one that is not a power of two.
        for size in [0u32, 130, 1024] {
            let mut d = disk();
            d[512 + 84..512 + 88].copy_from_slice(&size.to_le_bytes());
            reseal(&mut d);
            assert_eq!(parse(&d), Err(GptError::Header("entry size")));
        }
        // An entry table beyond the disk.
        let mut d = disk();
        d[512 + 72..512 + 80].copy_from_slice(&u64::MAX.to_le_bytes());
        reseal(&mut d);
        assert_eq!(parse(&d), Err(GptError::Header("entry table address")));
        // A header size beyond the sector.
        let mut d = disk();
        d[512 + 12..512 + 16].copy_from_slice(&4096u32.to_le_bytes());
        assert_eq!(parse(&d), Err(GptError::Header("size")));
        // Partitions that end beyond the disk, or before they start.
        let mut d = build(
            SECTORS,
            &[(ESP, 64, SECTORS), (OTHER, 900, 800), (OTHER, 0, 5)],
        );
        d.resize(usize::try_from(SECTORS * SECTOR).unwrap(), 0);
        assert_eq!(parse(&d), Ok(Vec::new()));
    }
}
