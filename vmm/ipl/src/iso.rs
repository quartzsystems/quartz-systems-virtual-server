// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Reading one file from an ISO 9660 volume: the installation media in
//! a partition's optical device, for IPL source D. Read-only; the
//! primary volume descriptor's tree, or the Joliet tree when there is
//! one. The image is the operator's, but it may have been made by
//! anyone, so nothing in it is trusted: the volume descriptors are
//! looked through up to a fixed count, a directory is read up to a
//! fixed size, every extent is checked against the image before it is
//! read, a file larger than the caller allows is refused before a byte
//! of it is read, and a file in more than one extent is refused.

use std::io::{Read, Seek, SeekFrom};

/// Bytes of a logical block: the only size taken.
pub const BLOCK: u64 = 2048;
/// The first volume descriptor's block.
const FIRST_DESCRIPTOR: u64 = 16;
/// The most volume descriptors looked at.
const MAX_DESCRIPTORS: u64 = 32;
/// The most bytes of one directory that are looked through.
const MAX_DIR_BYTES: u64 = 1 << 20;
/// The most directories a path may go through.
const MAX_DEPTH: usize = 8;
/// A directory record's fixed part.
const RECORD_HEAD: usize = 33;

const FLAG_DIRECTORY: u8 = 0x02;
const FLAG_MULTI_EXTENT: u8 = 0x80;

/// Why a file could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IsoError {
    /// The image could not be read.
    #[error("the image could not be read")]
    Io,
    /// Not an ISO 9660 volume.
    #[error("not an ISO 9660 volume: {0}")]
    NotIso(&'static str),
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

/// An extent: where, and how many bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Extent {
    block: u64,
    bytes: u64,
}

/// An ISO 9660 volume on `D`, an image of `bytes`.
pub struct Volume<D> {
    dev: D,
    bytes: u64,
    root: Extent,
    joliet: bool,
    /// The volume identifier of the primary descriptor.
    label: String,
}

fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The root directory's extent from a descriptor.
fn root_of(descriptor: &[u8]) -> Result<Extent, IsoError> {
    let record = &descriptor[156..156 + 34];
    if usize::from(record[0]) < RECORD_HEAD || record[25] & FLAG_DIRECTORY == 0 {
        return Err(IsoError::Damaged("the root directory record"));
    }
    Ok(Extent {
        block: u64::from(u32_le(record, 2)),
        bytes: u64::from(u32_le(record, 10)),
    })
}

/// A name of a directory record, as compared: upper case, without a
/// version (`;1`) or a trailing dot.
fn plain(name: &str) -> String {
    let name = name.split(';').next().unwrap_or("");
    name.strip_suffix('.').unwrap_or(name).to_ascii_uppercase()
}

impl<D: Read + Seek> Volume<D> {
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), IsoError> {
        let end = offset
            .checked_add(buf.len() as u64)
            .ok_or(IsoError::Damaged("an extent beyond the image"))?;
        if end > self.bytes {
            return Err(IsoError::Damaged("an extent beyond the image"));
        }
        self.dev
            .seek(SeekFrom::Start(offset))
            .map_err(|_| IsoError::Io)?;
        self.dev.read_exact(buf).map_err(|_| IsoError::Io)
    }

    /// The volume on `dev`, an image of `bytes`.
    pub fn open(dev: D, bytes: u64) -> Result<Self, IsoError> {
        let mut volume = Self {
            dev,
            bytes,
            root: Extent { block: 0, bytes: 0 },
            joliet: false,
            label: String::new(),
        };
        let mut primary = None;
        let mut joliet = None;
        let mut d = [0u8; 2048];
        for n in 0..MAX_DESCRIPTORS {
            let at = (FIRST_DESCRIPTOR + n) * BLOCK;
            if at + BLOCK > bytes {
                break;
            }
            volume.read_at(at, &mut d)?;
            if &d[1..6] != b"CD001" || d[6] != 1 {
                if n == 0 {
                    return Err(IsoError::NotIso("no volume descriptor"));
                }
                break;
            }
            match d[0] {
                1 if primary.is_none() => {
                    if u16::from_le_bytes([d[128], d[129]]) != 2048 {
                        return Err(IsoError::NotIso("a block size other than 2048"));
                    }
                    primary = Some(root_of(&d)?);
                    let label: String = d[40..72]
                        .iter()
                        .map(|b| char::from(*b))
                        .collect::<String>()
                        .trim_end()
                        .to_string();
                    volume.label = label;
                }
                // A supplementary descriptor with Joliet's escape.
                2 if joliet.is_none()
                    && d[88] == b'%'
                    && d[89] == b'/'
                    && matches!(d[90], b'@' | b'C' | b'E') =>
                {
                    joliet = root_of(&d).ok();
                }
                255 => break,
                _ => {}
            }
        }
        let root = primary.ok_or(IsoError::NotIso("no primary volume descriptor"))?;
        volume.root = root;
        if let Some(j) = joliet {
            volume.root = j;
            volume.joliet = true;
        }
        Ok(volume)
    }

    /// The volume identifier.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The name of a directory record.
    fn name_of(&self, raw: &[u8]) -> String {
        if self.joliet {
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            raw.iter().map(|b| char::from(*b)).collect()
        }
    }

    /// The entry named `name` in the directory at `dir`: whether it is
    /// a directory, and its extent.
    fn find(&mut self, dir: Extent, name: &str) -> Result<(bool, Extent), IsoError> {
        if dir.bytes > MAX_DIR_BYTES {
            return Err(IsoError::Damaged("a directory too large"));
        }
        let len = usize::try_from(dir.bytes).map_err(|_| IsoError::TooLarge)?;
        let mut bytes = vec![0u8; len];
        self.read_at(dir.block.saturating_mul(BLOCK), &mut bytes)?;
        let wanted = plain(name);
        let mut at = 0;
        while at < bytes.len() {
            let record_len = usize::from(bytes[at]);
            if record_len == 0 {
                // The rest of this block is padding.
                at = (at / 2048 + 1) * 2048;
                continue;
            }
            let Some(record) = bytes.get(at..at + record_len) else {
                return Err(IsoError::Damaged("a directory record beyond its directory"));
            };
            if record_len < RECORD_HEAD {
                return Err(IsoError::Damaged("a directory record too short"));
            }
            let name_len = usize::from(record[32]);
            let Some(raw) = record.get(RECORD_HEAD..RECORD_HEAD + name_len) else {
                return Err(IsoError::Damaged("a name beyond its record"));
            };
            // `\0` and `\1` are the directory itself and its parent.
            let special = name_len == 1 && raw[0] <= 1;
            if !special && plain(&self.name_of(raw)) == wanted {
                let flags = record[25];
                if flags & FLAG_MULTI_EXTENT != 0 {
                    return Err(IsoError::Damaged("a file in more than one extent"));
                }
                return Ok((
                    flags & FLAG_DIRECTORY != 0,
                    Extent {
                        block: u64::from(u32_le(record, 2)),
                        bytes: u64::from(u32_le(record, 10)),
                    },
                ));
            }
            at += record_len;
        }
        Err(IsoError::NotFound)
    }

    /// The bytes of the file at `path` (`EFI/BOOT/BOOTX64.EFI`), when
    /// it has no more than `max_bytes`. Names are compared without
    /// case and without a version.
    pub fn read_file(&mut self, path: &str, max_bytes: usize) -> Result<Vec<u8>, IsoError> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || parts.len() > MAX_DEPTH {
            return Err(IsoError::NotFound);
        }
        let mut dir = self.root;
        for (i, part) in parts.iter().enumerate() {
            let (is_dir, extent) = self.find(dir, part)?;
            if i + 1 < parts.len() {
                if !is_dir {
                    return Err(IsoError::NotFound);
                }
                dir = extent;
                continue;
            }
            if is_dir {
                return Err(IsoError::NotFound);
            }
            let size = usize::try_from(extent.bytes).map_err(|_| IsoError::TooLarge)?;
            if size > max_bytes {
                return Err(IsoError::TooLarge);
            }
            let mut out = vec![0u8; size];
            self.read_at(extent.block.saturating_mul(BLOCK), &mut out)?;
            return Ok(out);
        }
        Err(IsoError::NotFound)
    }
}

/// Whether `head`, the image's first 34 KiB or more, starts an ISO 9660
/// volume: a primary volume descriptor at block 16.
pub fn is_iso(head: &[u8]) -> bool {
    let at = usize::try_from(FIRST_DESCRIPTOR * BLOCK).unwrap_or(usize::MAX);
    head.get(at..at + 7)
        .is_some_and(|d| d[0] == 1 && &d[1..6] == b"CD001" && d[6] == 1)
}

/// Build a small image holding `files`, for the tests and the fuzzer's
/// corpus: a primary descriptor only, names as given (upper case), a
/// block a directory.
#[allow(clippy::too_many_lines)] // one image, written field by field
pub fn build(label: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    // Each directory, by path, with its entries.
    struct Dir {
        path: String,
        entries: Vec<(String, bool, usize)>,
    }
    let mut dirs = vec![Dir {
        path: String::new(),
        entries: Vec::new(),
    }];
    let mut blobs: Vec<&[u8]> = Vec::new();
    for (path, bytes) in files {
        let parts: Vec<&str> = path.split('/').collect();
        let mut parent = String::new();
        for (i, part) in parts.iter().enumerate() {
            let here = if parent.is_empty() {
                (*part).to_string()
            } else {
                format!("{parent}/{part}")
            };
            let leaf = i + 1 == parts.len();
            let p = dirs.iter().position(|d| d.path == parent).unwrap_or(0);
            if leaf {
                dirs[p]
                    .entries
                    .push(((*part).to_string(), false, blobs.len()));
                blobs.push(bytes);
            } else if !dirs.iter().any(|d| d.path == here) {
                let index = dirs.len();
                dirs[p].entries.push(((*part).to_string(), true, index));
                dirs.push(Dir {
                    path: here.clone(),
                    entries: Vec::new(),
                });
            }
            parent = here;
        }
    }
    // Blocks: 0-15 system area, 16 primary, 17 terminator, then the
    // directories, then the files.
    let block = usize::try_from(BLOCK).unwrap_or(2048);
    let dir_block = |i: usize| 18 + i;
    let mut file_block = Vec::new();
    let mut next = 18 + dirs.len();
    for b in &blobs {
        file_block.push(next);
        next += b.len().div_ceil(block).max(1);
    }
    let mut img = vec![0u8; next * block];
    let record = |out: &mut Vec<u8>, name: &[u8], at: usize, len: usize, dir: bool| {
        let n = RECORD_HEAD + name.len() + (name.len() + 1) % 2;
        let mut r = vec![0u8; n];
        r[0] = u8::try_from(n).unwrap_or(0);
        let at = u32::try_from(at).unwrap_or(0);
        let len = u32::try_from(len).unwrap_or(0);
        r[2..6].copy_from_slice(&at.to_le_bytes());
        r[6..10].copy_from_slice(&at.to_be_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        r[14..18].copy_from_slice(&len.to_be_bytes());
        r[25] = if dir { FLAG_DIRECTORY } else { 0 };
        r[28] = 1;
        r[32] = u8::try_from(name.len()).unwrap_or(0);
        r[RECORD_HEAD..RECORD_HEAD + name.len()].copy_from_slice(name);
        out.extend_from_slice(&r);
    };
    for (i, d) in dirs.iter().enumerate() {
        let mut out = Vec::new();
        record(&mut out, &[0], dir_block(i), block, true);
        record(&mut out, &[1], dir_block(0), block, true);
        for (name, is_dir, index) in &d.entries {
            if *is_dir {
                record(&mut out, name.as_bytes(), dir_block(*index), block, true);
            } else {
                let versioned = format!("{name};1");
                let len = blobs[*index].len();
                record(
                    &mut out,
                    versioned.as_bytes(),
                    file_block[*index],
                    len,
                    false,
                );
            }
        }
        let at = dir_block(i) * block;
        let n = out.len().min(block);
        img[at..at + n].copy_from_slice(&out[..n]);
    }
    for (i, b) in blobs.iter().enumerate() {
        let at = file_block[i] * block;
        img[at..at + b.len()].copy_from_slice(b);
    }
    let pvd = &mut img[16 * block..17 * block];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    let mut id = [b' '; 32];
    let n = label.len().min(32);
    id[..n].copy_from_slice(&label.as_bytes()[..n]);
    pvd[40..72].copy_from_slice(&id);
    let blocks = u32::try_from(next).unwrap_or(0);
    pvd[80..84].copy_from_slice(&blocks.to_le_bytes());
    pvd[84..88].copy_from_slice(&blocks.to_be_bytes());
    pvd[128..130].copy_from_slice(&2048u16.to_le_bytes());
    pvd[130..132].copy_from_slice(&2048u16.to_be_bytes());
    let mut root = Vec::new();
    record(&mut root, &[0], dir_block(0), block, true);
    pvd[156..156 + 34].copy_from_slice(&root[..34]);
    let term = &mut img[17 * block..18 * block];
    term[0] = 255;
    term[1..6].copy_from_slice(b"CD001");
    term[6] = 1;
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn image() -> Vec<u8> {
        build(
            "QS400_V1R1M0",
            &[
                ("EFI/BOOT/BOOTX64.EFI", b"the media UKI".as_slice()),
                ("MANIFEST.JSO", b"{}".as_slice()),
            ],
        )
    }

    fn open(img: Vec<u8>) -> Result<Volume<Cursor<Vec<u8>>>, IsoError> {
        let len = img.len() as u64;
        Volume::open(Cursor::new(img), len)
    }

    #[test]
    fn a_file_is_read_by_its_path_without_case_or_version() {
        let img = image();
        assert!(is_iso(&img));
        let mut v = open(img).unwrap();
        assert_eq!(v.label(), "QS400_V1R1M0");
        assert_eq!(
            v.read_file("efi/boot/bootx64.efi", 1 << 20).unwrap(),
            b"the media UKI"
        );
        assert_eq!(v.read_file("/MANIFEST.JSO", 10).unwrap(), b"{}");
        assert_eq!(v.read_file("EFI/BOOT", 10), Err(IsoError::NotFound));
        assert_eq!(v.read_file("EFI/NONE.EFI", 10), Err(IsoError::NotFound));
        assert_eq!(v.read_file("MANIFEST.JSO/X", 10), Err(IsoError::NotFound));
        assert_eq!(
            v.read_file("EFI/BOOT/BOOTX64.EFI", 4),
            Err(IsoError::TooLarge)
        );
    }

    #[test]
    fn what_is_not_an_iso_is_refused() {
        assert!(matches!(open(vec![0; 64 * 1024]), Err(IsoError::NotIso(_))));
        assert!(matches!(open(vec![0; 100]), Err(IsoError::NotIso(_))));
        let mut img = image();
        img[16 * 2048 + 129] = 4; // a block size of 1024
        assert!(matches!(open(img), Err(IsoError::NotIso(_))));
        assert!(!is_iso(&vec![0; 40_000]));
    }

    #[test]
    fn extents_beyond_the_image_are_refused_not_read() {
        let mut img = image();
        // The file's extent: far past the end.
        let len = img.len();
        let root = 18 * 2048;
        let efi = u32_le(&img, root + 34 + 34 + 2) as usize * 2048;
        let boot = u32_le(&img, efi + 34 + 34 + 2) as usize * 2048;
        let file = boot + 34 + 34;
        img[file + 2..file + 6].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        let mut v = open(img.clone()).unwrap();
        assert!(matches!(
            v.read_file("EFI/BOOT/BOOTX64.EFI", 1 << 20),
            Err(IsoError::Damaged(_))
        ));
        // A record that says it is longer than its directory.
        img[file] = 255;
        img.truncate(len);
        let mut v = open(img.clone()).unwrap();
        assert!(v.read_file("EFI/BOOT/BOOTX64.EFI", 1 << 20).is_err());
        // A directory that says it is huge.
        let mut huge = image();
        huge[root + 34 + 34 + 10..root + 34 + 34 + 14].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut v = open(huge).unwrap();
        assert!(matches!(
            v.read_file("EFI/BOOT/BOOTX64.EFI", 1 << 20),
            Err(IsoError::Damaged(_))
        ));
    }

    #[test]
    fn a_file_in_more_than_one_extent_is_refused() {
        let mut img = image();
        let root = 18 * 2048;
        // MANIFEST.JSO is the root's fourth record.
        let efi_len = usize::from(img[root + 68]);
        let manifest = root + 68 + efi_len;
        img[manifest + 25] |= FLAG_MULTI_EXTENT;
        let mut v = open(img).unwrap();
        assert!(matches!(
            v.read_file("MANIFEST.JSO", 10),
            Err(IsoError::Damaged(_))
        ));
    }
}
