// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Reading one file from the FAT volume of a guest's ESP. Read-only,
//! FAT16 and FAT32. The volume is the guest's to write, so nothing in
//! it is trusted: a cluster chain is followed for as many clusters as
//! the file's size needs and no further, a directory is read up to a
//! fixed size, a file larger than the caller allows is refused before
//! a byte of it is read, and every cluster number is checked against
//! the volume before it becomes an offset.

use std::io::{Read, Seek, SeekFrom};

/// The most bytes of one directory that are looked through.
const MAX_DIR_BYTES: usize = 1 << 20;
const ENTRY: usize = 32;
const ATTR_LONG_NAME: u8 = 0x0F;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;

/// Why a file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FatError {
    /// The volume could not be read.
    #[error("the volume could not be read")]
    Io,
    /// Not a FAT16 or FAT32 volume.
    #[error("not a FAT volume: {0}")]
    NotFat(&'static str),
    /// No such file.
    #[error("no such file")]
    NotFound,
    /// The file is larger than allowed.
    #[error("the file is larger than allowed")]
    TooLarge,
    /// The volume's structures contradict themselves.
    #[error("the volume is damaged: {0}")]
    Damaged(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Fat16,
    Fat32,
}

#[derive(Clone, Copy)]
enum Dir {
    /// The fixed root directory of FAT16: where, and how many bytes.
    Root(u64, usize),
    /// A directory in a cluster chain.
    Chain(u32),
}

/// A FAT volume on `D`, which is the partition.
pub struct Volume<D> {
    dev: D,
    kind: Kind,
    cluster_bytes: u64,
    fat_start: u64,
    data_start: u64,
    clusters: u32,
    root: Dir,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// An 8.3 name as it is written: `BOOTX64.EFI`.
fn short_name(entry: &[u8]) -> String {
    let part = |bytes: &[u8]| -> String {
        let end = bytes.iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
        bytes[..end].iter().map(|b| char::from(*b)).collect()
    };
    let (base, ext) = (part(&entry[..8]), part(&entry[8..11]));
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// The thirteen characters of one long-name entry.
fn long_part(entry: &[u8]) -> Vec<u16> {
    [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30]
        .iter()
        .map(|at| u16_at(entry, *at))
        .take_while(|c| *c != 0 && *c != 0xFFFF)
        .collect()
}

impl<D: Read + Seek> Volume<D> {
    /// The volume on `dev`, a partition of `bytes`.
    pub fn open(mut dev: D, bytes: u64) -> Result<Self, FatError> {
        let mut b = [0u8; 512];
        dev.seek(SeekFrom::Start(0)).map_err(|_| FatError::Io)?;
        dev.read_exact(&mut b).map_err(|_| FatError::Io)?;
        if b[510..] != [0x55, 0xAA] {
            return Err(FatError::NotFat("no boot signature"));
        }
        let sector = u64::from(u16_at(&b, 11));
        if !matches!(sector, 512 | 1024 | 2048 | 4096) {
            return Err(FatError::NotFat("sector size"));
        }
        let per_cluster = u64::from(b[13]);
        if per_cluster == 0 || !per_cluster.is_power_of_two() {
            return Err(FatError::NotFat("cluster size"));
        }
        let reserved = u64::from(u16_at(&b, 14));
        let fats = u64::from(b[16]);
        if reserved == 0 || !(1..=2).contains(&fats) {
            return Err(FatError::NotFat("reserved sectors or FAT count"));
        }
        let root_entries = usize::from(u16_at(&b, 17));
        let total = match u16_at(&b, 19) {
            0 => u64::from(u32_at(&b, 32)),
            n => u64::from(n),
        };
        let fat_sectors = match u16_at(&b, 22) {
            0 => u64::from(u32_at(&b, 36)),
            n => u64::from(n),
        };
        // Every factor is below 2^32 and every product below 2^48.
        let root_sectors = (root_entries as u64 * ENTRY as u64).div_ceil(sector);
        let data_sector = reserved + fats * fat_sectors + root_sectors;
        if total == 0 || total * sector > bytes || data_sector >= total || fat_sectors == 0 {
            return Err(FatError::NotFat("geometry"));
        }
        let clusters = (total - data_sector) / per_cluster;
        let kind = match clusters {
            0..4085 => return Err(FatError::NotFat("FAT12")),
            4085..65525 => Kind::Fat16,
            _ => Kind::Fat32,
        };
        let clusters = u32::try_from(clusters).map_err(|_| FatError::NotFat("geometry"))?;
        let entry_bytes = if kind == Kind::Fat16 { 2 } else { 4 };
        if fat_sectors * sector < (u64::from(clusters) + 2) * entry_bytes {
            return Err(FatError::Damaged("the FAT is shorter than the volume"));
        }
        let root = match kind {
            Kind::Fat16 => Dir::Root(
                (reserved + fats * fat_sectors) * sector,
                root_entries * ENTRY,
            ),
            Kind::Fat32 => Dir::Chain(u32_at(&b, 44)),
        };
        Ok(Self {
            dev,
            kind,
            cluster_bytes: per_cluster * sector,
            fat_start: reserved * sector,
            data_start: data_sector * sector,
            clusters,
            root,
        })
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), FatError> {
        self.dev
            .seek(SeekFrom::Start(offset))
            .map_err(|_| FatError::Io)?;
        self.dev.read_exact(buf).map_err(|_| FatError::Io)
    }

    /// Whether `cluster` is one of the volume's.
    fn valid(&self, cluster: u32) -> bool {
        cluster >= 2 && cluster - 2 < self.clusters
    }

    /// The cluster after `cluster`; `None` at the chain's end.
    fn next(&mut self, cluster: u32) -> Result<Option<u32>, FatError> {
        let next = match self.kind {
            Kind::Fat16 => {
                let mut b = [0u8; 2];
                self.read_at(self.fat_start + u64::from(cluster) * 2, &mut b)?;
                match u16::from_le_bytes(b) {
                    0xFFF8.. => return Ok(None),
                    n => u32::from(n),
                }
            }
            Kind::Fat32 => {
                let mut b = [0u8; 4];
                self.read_at(self.fat_start + u64::from(cluster) * 4, &mut b)?;
                match u32::from_le_bytes(b) & 0x0FFF_FFFF {
                    0x0FFF_FFF8.. => return Ok(None),
                    n => n,
                }
            }
        };
        if self.valid(next) {
            Ok(Some(next))
        } else {
            Err(FatError::Damaged("a cluster chain leaves the volume"))
        }
    }

    /// `bytes` of the chain that starts at `first`.
    fn read_chain(&mut self, first: u32, bytes: usize) -> Result<Vec<u8>, FatError> {
        let mut out = vec![0u8; bytes];
        let mut cluster = first;
        let cluster_bytes = usize::try_from(self.cluster_bytes).map_err(|_| FatError::Io)?;
        let mut pieces = out.chunks_mut(cluster_bytes).peekable();
        while let Some(piece) = pieces.next() {
            if !self.valid(cluster) {
                return Err(FatError::Damaged("a cluster chain leaves the volume"));
            }
            let offset = self.data_start + u64::from(cluster - 2) * self.cluster_bytes;
            self.read_at(offset, piece)?;
            if pieces.peek().is_some() {
                cluster = self
                    .next(cluster)?
                    .ok_or(FatError::Damaged("a cluster chain ends before the file"))?;
            }
        }
        Ok(out)
    }

    /// A directory's entries, up to [`MAX_DIR_BYTES`].
    fn read_dir(&mut self, dir: Dir) -> Result<Vec<u8>, FatError> {
        match dir {
            Dir::Root(offset, bytes) => {
                let mut out = vec![0u8; bytes.min(MAX_DIR_BYTES)];
                self.read_at(offset, &mut out)?;
                Ok(out)
            }
            Dir::Chain(first) => {
                let cluster_bytes =
                    usize::try_from(self.cluster_bytes).map_err(|_| FatError::Io)?;
                let mut out = Vec::new();
                let mut cluster = Some(first);
                while let Some(c) = cluster {
                    if !self.valid(c) {
                        return Err(FatError::Damaged("a directory leaves the volume"));
                    }
                    if out.len() + cluster_bytes > MAX_DIR_BYTES {
                        break;
                    }
                    let at = out.len();
                    out.resize(at + cluster_bytes, 0);
                    let offset = self.data_start + u64::from(c - 2) * self.cluster_bytes;
                    self.read_at(offset, &mut out[at..])?;
                    cluster = self.next(c)?;
                }
                Ok(out)
            }
        }
    }

    /// The entry called `name` in `dir`: whether it is a directory,
    /// its first cluster and its size.
    fn find(&mut self, dir: Dir, name: &str) -> Result<(bool, u32, u32), FatError> {
        let entries = self.read_dir(dir)?;
        let mut long: Vec<(u8, Vec<u16>)> = Vec::new();
        for e in entries.as_chunks::<ENTRY>().0 {
            match (e[0], e[11]) {
                (0x00, _) => break,
                (0xE5, _) => long.clear(),
                (order, ATTR_LONG_NAME) => {
                    if long.len() < 20 {
                        long.push((order & 0x1F, long_part(e)));
                    }
                }
                (_, attributes) if attributes & ATTR_VOLUME != 0 => long.clear(),
                (_, attributes) => {
                    // The parts are stored last first.
                    long.sort_by_key(|(order, _)| *order);
                    let long_name: Vec<u16> = long.drain(..).flat_map(|(_, part)| part).collect();
                    let long_name = String::from_utf16_lossy(&long_name);
                    if short_name(e).eq_ignore_ascii_case(name)
                        || (!long_name.is_empty() && long_name.eq_ignore_ascii_case(name))
                    {
                        let cluster = (u32::from(u16_at(e, 20)) << 16) | u32::from(u16_at(e, 26));
                        let cluster = match self.kind {
                            Kind::Fat16 => cluster & 0xFFFF,
                            Kind::Fat32 => cluster,
                        };
                        return Ok((attributes & ATTR_DIRECTORY != 0, cluster, u32_at(e, 28)));
                    }
                }
            }
        }
        Err(FatError::NotFound)
    }

    /// The bytes of the file at `path` (`EFI/BOOT/BOOTX64.EFI`), when
    /// it has no more than `max_bytes`.
    pub fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FatError> {
        let mut dir = self.root;
        let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
        while let Some(part) = parts.next() {
            let (is_dir, cluster, size) = self.find(dir, part)?;
            if parts.peek().is_some() {
                if !is_dir {
                    return Err(FatError::NotFound);
                }
                dir = Dir::Chain(cluster);
                continue;
            }
            if is_dir {
                return Err(FatError::NotFound);
            }
            let size = usize::try_from(size).map_err(|_| FatError::TooLarge)?;
            if size > max_bytes {
                return Err(FatError::TooLarge);
            }
            return self.read_chain(cluster, size);
        }
        Err(FatError::NotFound)
    }
}

/// Build a small volume holding `files`, FAT16 or FAT32, for the tests
/// and the fuzzer's corpus. One cluster a directory; names are 8.3.
#[allow(clippy::too_many_lines)] // one volume, written field by field
pub fn build(fat32: bool, files: &[(&str, &[u8])]) -> Vec<u8> {
    const SECTOR: usize = 512;
    let clusters: usize = if fat32 { 66_000 } else { 5_000 };
    let entry = if fat32 { 4 } else { 2 };
    let fat_sectors = ((clusters + 2) * entry).div_ceil(SECTOR);
    let root_entries = if fat32 { 0 } else { 32 };
    let reserved = 1;
    let fat_start = reserved * SECTOR;
    let root_start = fat_start + fat_sectors * SECTOR;
    let data_start = root_start + root_entries * ENTRY;
    let total = data_start / SECTOR + clusters;
    let mut v = vec![0u8; total * SECTOR];
    v[11..13].copy_from_slice(&512u16.to_le_bytes());
    v[13] = 1;
    v[14..16].copy_from_slice(&1u16.to_le_bytes());
    v[16] = 1;
    v[17..19].copy_from_slice(&u16::try_from(root_entries).unwrap_or(0).to_le_bytes());
    v[32..36].copy_from_slice(&u32::try_from(total).unwrap_or(0).to_le_bytes());
    if fat32 {
        v[36..40].copy_from_slice(&u32::try_from(fat_sectors).unwrap_or(0).to_le_bytes());
        v[44..48].copy_from_slice(&2u32.to_le_bytes());
    } else {
        v[22..24].copy_from_slice(&u16::try_from(fat_sectors).unwrap_or(0).to_le_bytes());
    }
    v[510] = 0x55;
    v[511] = 0xAA;

    let mut next_cluster = if fat32 { 3 } else { 2 };
    let set_fat = |v: &mut Vec<u8>, cluster: usize, value: u32| {
        let at = fat_start + cluster * entry;
        v[at..at + entry].copy_from_slice(&value.to_le_bytes()[..entry]);
    };
    let end = if fat32 { 0x0FFF_FFFF } else { 0xFFFF };
    if fat32 {
        set_fat(&mut v, 2, end);
    }
    let cluster_at = |cluster: usize| data_start + (cluster - 2) * SECTOR;
    // Where each directory's entries are, and how many it has.
    let root = if fat32 { cluster_at(2) } else { root_start };
    let mut dirs: Vec<(String, usize, usize)> = vec![(String::new(), root, 0)];
    let add_entry = |v: &mut Vec<u8>,
                     dirs: &mut Vec<(String, usize, usize)>,
                     dir: &str,
                     name: &str,
                     attr: u8,
                     cluster: usize,
                     size: usize| {
        let d = dirs
            .iter_mut()
            .find(|d| d.0 == dir)
            .expect("the directory exists");
        let at = d.1 + d.2 * ENTRY;
        d.2 += 1;
        let (base, ext) = name.split_once('.').unwrap_or((name, ""));
        let mut short = [b' '; 11];
        short[..base.len()].copy_from_slice(base.as_bytes());
        short[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
        v[at..at + 11].copy_from_slice(&short);
        v[at + 11] = attr;
        let cluster = u32::try_from(cluster).unwrap_or(0);
        v[at + 20..at + 22].copy_from_slice(&cluster.to_le_bytes()[2..]);
        v[at + 26..at + 28].copy_from_slice(&cluster.to_le_bytes()[..2]);
        v[at + 28..at + 32].copy_from_slice(&u32::try_from(size).unwrap_or(0).to_le_bytes());
    };
    for (path, data) in files {
        let parts: Vec<&str> = path.split('/').collect();
        let mut dir = String::new();
        for part in &parts[..parts.len() - 1] {
            let child = if dir.is_empty() {
                (*part).to_string()
            } else {
                format!("{dir}/{part}")
            };
            if !dirs.iter().any(|d| d.0 == child) {
                let cluster = next_cluster;
                next_cluster += 1;
                set_fat(&mut v, cluster, end);
                add_entry(&mut v, &mut dirs, &dir, part, ATTR_DIRECTORY, cluster, 0);
                dirs.push((child.clone(), cluster_at(cluster), 0));
            }
            dir = child;
        }
        let first = next_cluster;
        let count = data.len().div_ceil(SECTOR).max(1);
        for i in 0..count {
            let value = if i + 1 == count {
                end
            } else {
                u32::try_from(first + i + 1).unwrap_or(end)
            };
            set_fat(&mut v, first + i, value);
        }
        next_cluster += count;
        let at = cluster_at(first);
        v[at..at + data.len()].copy_from_slice(data);
        add_entry(
            &mut v,
            &mut dirs,
            &dir,
            parts[parts.len() - 1],
            0x20,
            first,
            data.len(),
        );
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const A: &str = "EFI/BOOT/BOOTX64.EFI";
    const B: &str = "EFI/QS400/B.EFI";

    fn files() -> (Vec<u8>, Vec<u8>) {
        let a: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        let b: Vec<u8> = (0..700u32).map(|i| (i % 7) as u8).collect();
        (a, b)
    }

    fn volume(fat32: bool) -> Vec<u8> {
        let (a, b) = files();
        build(fat32, &[(A, &a), (B, &b)])
    }

    fn open(v: &[u8]) -> Result<Volume<Cursor<&[u8]>>, FatError> {
        Volume::open(Cursor::new(v), v.len() as u64)
    }

    #[test]
    fn both_sides_files_are_read_from_both_kinds_of_volume() {
        let (a, b) = files();
        for fat32 in [false, true] {
            let v = volume(fat32);
            let mut vol = open(&v).unwrap();
            assert_eq!(vol.kind == Kind::Fat32, fat32);
            assert_eq!(vol.read_file(A, 1 << 20).unwrap(), a);
            assert_eq!(vol.read_file(B, 1 << 20).unwrap(), b);
            // Names are matched as FAT matches them.
            assert_eq!(vol.read_file("/efi/qs400/b.efi", 1 << 20).unwrap(), b);
            assert_eq!(
                vol.read_file("EFI/QS400/C.EFI", 1 << 20),
                Err(FatError::NotFound)
            );
            assert_eq!(vol.read_file("EFI/BOOT", 1 << 20), Err(FatError::NotFound));
            assert_eq!(
                vol.read_file("EFI/BOOT/BOOTX64.EFI/X", 1 << 20),
                Err(FatError::NotFound)
            );
            assert_eq!(vol.read_file("", 1 << 20), Err(FatError::NotFound));
            assert_eq!(vol.read_file(A, 2999), Err(FatError::TooLarge));
        }
    }

    #[test]
    fn what_is_not_a_fat_volume_is_refused() {
        assert_eq!(open(&[0u8; 100]).err(), Some(FatError::Io));
        assert!(matches!(open(&[0u8; 4096]), Err(FatError::NotFat(_))));
        let good = volume(false);
        for (at, value) in [
            (11usize, 0x01u8),
            (13, 3),
            (13, 0),
            (16, 0),
            (16, 9),
            (14, 0),
        ] {
            let mut v = good.clone();
            v[at] = value;
            if at == 14 {
                v[15] = 0;
            }
            assert!(matches!(open(&v), Err(FatError::NotFat(_))), "byte {at}");
        }
        // A volume that says it is larger than its partition.
        let mut v = good.clone();
        v[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(open(&v), Err(FatError::NotFat("geometry"))));
        // A FAT too short for the clusters it should describe.
        let mut v = good;
        v[22..24].copy_from_slice(&1u16.to_le_bytes());
        assert!(open(&v).is_err());
    }

    #[test]
    fn a_chain_that_loops_or_leaves_the_volume_ends_the_read() {
        let (a, _) = files();
        let v = volume(false);
        let fat = 512;
        // The file's first cluster: after the three directories.
        let first = 2 + 2;
        let mut vol = open(&v).unwrap();
        assert_eq!(vol.read_file(A, 1 << 20).unwrap(), a);

        // A chain that points back at itself is followed for the
        // file's six clusters and no further.
        let mut looped = v.clone();
        looped[fat + first * 2..fat + first * 2 + 2]
            .copy_from_slice(&u16::try_from(first).unwrap().to_le_bytes());
        let got = open(&looped).unwrap().read_file(A, 1 << 20).unwrap();
        assert_eq!(got.len(), a.len());
        assert_eq!(got[..512], a[..512]);

        // A chain that leaves the volume.
        let mut wild = v.clone();
        wild[fat + first * 2..fat + first * 2 + 2].copy_from_slice(&0xFFF0u16.to_le_bytes());
        assert!(matches!(
            open(&wild).unwrap().read_file(A, 1 << 20),
            Err(FatError::Damaged(_))
        ));

        // A chain that ends before the file does.
        let mut short = v.clone();
        short[fat + first * 2..fat + first * 2 + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
        assert!(matches!(
            open(&short).unwrap().read_file(A, 1 << 20),
            Err(FatError::Damaged(_))
        ));

        // A size no file here may have is refused before any read.
        let mut huge = v;
        let root = 512 + 20 * 512;
        // EFI is the root's first entry; its directory is cluster 2.
        assert_eq!(&huge[root..root + 3], b"EFI");
        let boot_dir = root + 32 * 32 + 512;
        assert_eq!(&huge[boot_dir..boot_dir + 7], b"BOOTX64");
        huge[boot_dir + 28..boot_dir + 32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            open(&huge).unwrap().read_file(A, 64 << 20),
            Err(FatError::TooLarge)
        );
    }

    #[test]
    fn a_directory_that_loops_is_read_to_its_bound() {
        let mut v = volume(true);
        let fat = 512;
        // The root directory of FAT32 is cluster 2: make it its own next.
        v[fat + 8..fat + 12].copy_from_slice(&2u32.to_le_bytes());
        let mut vol = open(&v).unwrap();
        assert_eq!(vol.read_file("NOSUCH", 1 << 20), Err(FatError::NotFound));
        // A root cluster that is not on the volume.
        v[44..48].copy_from_slice(&0x0FFF_FFF0u32.to_le_bytes());
        assert!(matches!(
            open(&v).unwrap().read_file(A, 1 << 20),
            Err(FatError::Damaged(_))
        ));
    }

    #[test]
    fn long_names_are_matched_too() {
        let mut v = build(false, &[("KERNEL.EFI", b"long")]);
        let root = 512 + 20 * 512;
        // Move the short entry down and put a long-name entry before it.
        let short: Vec<u8> = v[root..root + 32].to_vec();
        v[root + 32..root + 64].copy_from_slice(&short);
        let mut long = [0xFFu8; 32];
        long[0] = 0x41;
        long[11] = ATTR_LONG_NAME;
        long[12] = 0;
        long[26] = 0;
        long[27] = 0;
        let name: Vec<u16> = "Side-A.uki".encode_utf16().chain([0]).collect();
        for (c, at) in name
            .iter()
            .zip([1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30])
        {
            long[at..at + 2].copy_from_slice(&c.to_le_bytes());
        }
        v[root..root + 32].copy_from_slice(&long);
        let mut vol = open(&v).unwrap();
        assert_eq!(vol.read_file("side-a.UKI", 100).unwrap(), b"long");
        assert_eq!(vol.read_file("KERNEL.EFI", 100).unwrap(), b"long");
    }
}
