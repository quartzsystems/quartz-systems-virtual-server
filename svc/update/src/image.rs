// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An update image's container (`docs/UPDATE.md`): one file, its
//! signed manifest first, then the files it names.
//!
//! ```text
//! "QSVSUPDT"            8 bytes
//! version               u32, little-endian: 1
//! manifest length       u32
//! signature length      u32: 64, ed25519
//! files                 u32
//! manifest              its text
//! signature             over the manifest's bytes
//! each file             name length u16, name, size u64, its bytes
//! ```
//!
//! Read as hostile: every length bounded before it is used, nothing
//! allocated by a size the image names, the files' ranges inside it.

use std::ops::Range;

use crate::manifest::{MAX_ITEMS, MAX_TEXT, MAX_WORD};

/// The first bytes.
pub const MAGIC: &[u8; 8] = b"QSVSUPDT";
/// The container's version.
pub const VERSION: u32 = 1;
/// An ed25519 signature's length.
pub const SIGNATURE: usize = 64;
/// The largest image: two UKIs of at most 128 MiB and a manifest.
pub const MAX_IMAGE: u64 = 300 * 1024 * 1024;

/// An image read: where each part is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    /// The manifest's bytes.
    pub manifest: Range<usize>,
    /// The signature's.
    pub signature: Range<usize>,
    /// Each file: its name and its bytes' place.
    pub files: Vec<(String, Range<usize>)>,
}

/// Why bytes are not an image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Damaged(pub String);

fn damaged(why: impl Into<String>) -> Damaged {
    Damaged(why.into())
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize, what: &str) -> Result<Range<usize>, Damaged> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.bytes.len())
            .ok_or_else(|| damaged(format!("the image ends inside its {what}")))?;
        let r = self.at..end;
        self.at = end;
        Ok(r)
    }

    fn u16(&mut self, what: &str) -> Result<u16, Damaged> {
        let r = self.take(2, what)?;
        Ok(u16::from_le_bytes([
            self.bytes[r.start],
            self.bytes[r.start + 1],
        ]))
    }

    fn u32(&mut self, what: &str) -> Result<u32, Damaged> {
        let r = self.take(4, what)?;
        let mut b = [0u8; 4];
        b.copy_from_slice(&self.bytes[r]);
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self, what: &str) -> Result<u64, Damaged> {
        let r = self.take(8, what)?;
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.bytes[r]);
        Ok(u64::from_le_bytes(b))
    }
}

/// Where the parts of the image `bytes` are.
///
/// # Errors
/// It is not an image, or not a whole one.
pub fn parse(bytes: &[u8]) -> Result<Parsed, Damaged> {
    if bytes.len() as u64 > MAX_IMAGE {
        return Err(damaged("the image is larger than any can be"));
    }
    let mut r = Reader { bytes, at: 0 };
    let magic = r.take(MAGIC.len(), "magic")?;
    if &bytes[magic] != MAGIC {
        return Err(damaged("it is not an update image"));
    }
    if r.u32("version")? != VERSION {
        return Err(damaged("it is an update image of another version"));
    }
    let manifest_len = usize::try_from(r.u32("header")?).unwrap_or(usize::MAX);
    let signature_len = usize::try_from(r.u32("header")?).unwrap_or(usize::MAX);
    let count = usize::try_from(r.u32("header")?).unwrap_or(usize::MAX);
    if manifest_len > MAX_TEXT || signature_len != SIGNATURE || count > MAX_ITEMS {
        return Err(damaged("its header is not one of an update image"));
    }
    let manifest = r.take(manifest_len, "manifest")?;
    let signature = r.take(signature_len, "signature")?;
    let mut files = Vec::new();
    for _ in 0..count {
        let name_len = usize::from(r.u16("file's name")?);
        if name_len == 0 || name_len > MAX_WORD {
            return Err(damaged("a file's name is not one"));
        }
        let name = r.take(name_len, "file's name")?;
        let name = std::str::from_utf8(&bytes[name])
            .map_err(|_| damaged("a file's name is not text"))?
            .to_string();
        let size = usize::try_from(r.u64("file's size")?).unwrap_or(usize::MAX);
        let data = r.take(size, "file")?;
        files.push((name, data));
    }
    if r.at != bytes.len() {
        return Err(damaged("the image has bytes after its last file"));
    }
    Ok(Parsed {
        manifest,
        signature,
        files,
    })
}

/// The image of `manifest` (its text), `signature` and `files`.
#[must_use]
pub fn write(manifest: &str, signature: &[u8; SIGNATURE], files: &[(&str, &[u8])]) -> Vec<u8> {
    let size: usize = files.iter().map(|(n, d)| 10 + n.len() + d.len()).sum();
    let mut out = Vec::with_capacity(24 + manifest.len() + SIGNATURE + size);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    for n in [manifest.len(), SIGNATURE, files.len()] {
        out.extend_from_slice(&u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes());
    }
    out.extend_from_slice(manifest.as_bytes());
    out.extend_from_slice(signature);
    for (name, data) in files {
        out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_reads_back_and_a_damaged_one_does_not() {
        let img = write(
            "QSVS-UPDATE 1\n",
            &[7; SIGNATURE],
            &[("SLOTA.EFI", b"aaaa"), ("SLOTB.EFI", b"bb")],
        );
        let p = parse(&img).unwrap();
        assert_eq!(&img[p.manifest.clone()], b"QSVS-UPDATE 1\n");
        assert_eq!(&img[p.signature.clone()], &[7; SIGNATURE]);
        assert_eq!(p.files.len(), 2);
        assert_eq!(&img[p.files[1].1.clone()], b"bb");
        // Every cut, and a byte more, is refused, never a panic.
        for n in 0..img.len() {
            assert!(parse(&img[..n]).is_err(), "cut at {n}");
        }
        let mut longer = img.clone();
        longer.push(0);
        assert!(parse(&longer).is_err());
        let mut other = img.clone();
        other[0] = b'X';
        assert_eq!(parse(&other).unwrap_err().0, "it is not an update image");
        // A size that names more than there is.
        let mut huge = img.clone();
        let at = img.len() - 2 - 8;
        huge[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&huge).is_err());
    }
}
