// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A ZIP archive (PKWARE's APPNOTE, the parts every workstation's
//! archive tool reads): each entry stored, not compressed, with its
//! CRC-32; a central directory; the end record with the archive's
//! comment. No ZIP64: an archive and each entry stay under 4 GiB. The
//! same entries at the same time make the same bytes.

use std::io::{self, Read, Seek, SeekFrom, Write};

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// Version 2.0: stored entries, names in UTF-8 (bit 11).
const VERSION: u16 = 20;
const UTF8: u16 = 1 << 11;
/// The most an entry or the archive may be.
pub const MAX_BYTES: u64 = u32::MAX as u64;
/// The longest comment.
pub const MAX_COMMENT: usize = 4096;

/// CRC-32 (IEEE), as ZIP has it.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// A time as MS-DOS has it: (time, date), in UTC; 1980 at the least.
#[must_use]
pub fn dos_time(unix_secs: i64) -> (u16, u16) {
    let c = appliance::time::civil(unix_secs);
    if c.year < 1980 {
        return (0, (1 << 5) | 1);
    }
    let time = (c.hour << 11) | (c.minute << 5) | (c.second / 2);
    let year = u32::try_from(c.year.min(2107) - 1980).unwrap_or(0);
    let date = (year << 9) | (c.month << 5) | c.day;
    (
        u16::try_from(time).unwrap_or(0),
        u16::try_from(date).unwrap_or(0),
    )
}

struct Central {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

/// An archive being written.
pub struct Writer<W: Write> {
    out: W,
    at: u64,
    time: (u16, u16),
    entries: Vec<Central>,
}

fn too_big() -> io::Error {
    io::Error::other("the archive would be over 4 GiB")
}

impl<W: Write> Writer<W> {
    /// An archive written to `out`, every entry dated `unix_secs`.
    pub fn new(out: W, unix_secs: i64) -> Self {
        Self {
            out,
            at: 0,
            time: dos_time(unix_secs),
            entries: Vec::new(),
        }
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.at += bytes.len() as u64;
        if self.at > MAX_BYTES {
            return Err(too_big());
        }
        Ok(())
    }

    /// Bytes written so far.
    #[must_use]
    pub fn written(&self) -> u64 {
        self.at
    }

    /// Add an entry named `name` holding `data`.
    ///
    /// # Errors
    /// The output failed, or the archive would be over 4 GiB.
    pub fn add(&mut self, name: &str, data: &[u8]) -> io::Result<()> {
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let offset = u32::try_from(self.at).map_err(|_| too_big())?;
        let name_len =
            u16::try_from(name.len()).map_err(|_| io::Error::other("a name too long"))?;
        let crc = crc32(data);
        let mut h = Vec::with_capacity(30 + name.len());
        h.extend_from_slice(&LOCAL.to_le_bytes());
        h.extend_from_slice(&VERSION.to_le_bytes());
        h.extend_from_slice(&UTF8.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // stored
        h.extend_from_slice(&self.time.0.to_le_bytes());
        h.extend_from_slice(&self.time.1.to_le_bytes());
        h.extend_from_slice(&crc.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&name_len.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(name.as_bytes());
        self.put(&h)?;
        self.put(data)?;
        self.entries.push(Central {
            name: name.to_string(),
            crc,
            size,
            offset,
        });
        Ok(())
    }

    /// Write the central directory and the end record, with `comment`.
    ///
    /// # Errors
    /// The output failed, or the archive would be over 4 GiB.
    pub fn finish(mut self, comment: &str) -> io::Result<W> {
        let start = u32::try_from(self.at).map_err(|_| too_big())?;
        let entries = std::mem::take(&mut self.entries);
        for e in &entries {
            let mut c = Vec::with_capacity(46 + e.name.len());
            c.extend_from_slice(&CENTRAL.to_le_bytes());
            c.extend_from_slice(&VERSION.to_le_bytes()); // made by
            c.extend_from_slice(&VERSION.to_le_bytes()); // needed
            c.extend_from_slice(&UTF8.to_le_bytes());
            c.extend_from_slice(&0u16.to_le_bytes());
            c.extend_from_slice(&self.time.0.to_le_bytes());
            c.extend_from_slice(&self.time.1.to_le_bytes());
            c.extend_from_slice(&e.crc.to_le_bytes());
            c.extend_from_slice(&e.size.to_le_bytes());
            c.extend_from_slice(&e.size.to_le_bytes());
            c.extend_from_slice(&u16::try_from(e.name.len()).unwrap_or(0).to_le_bytes());
            c.extend_from_slice(&[0; 2 + 2 + 2 + 2]); // extra, comment, disk, internal
            c.extend_from_slice(&0u32.to_le_bytes()); // external attributes
            c.extend_from_slice(&e.offset.to_le_bytes());
            c.extend_from_slice(e.name.as_bytes());
            self.put(&c)?;
        }
        let size = u32::try_from(self.at - u64::from(start)).map_err(|_| too_big())?;
        let count =
            u16::try_from(entries.len()).map_err(|_| io::Error::other("too many entries"))?;
        let comment = &comment.as_bytes()[..comment.len().min(MAX_COMMENT)];
        let mut end = Vec::with_capacity(22 + comment.len());
        end.extend_from_slice(&END.to_le_bytes());
        end.extend_from_slice(&[0; 4]); // this disk, the directory's disk
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&count.to_le_bytes());
        end.extend_from_slice(&size.to_le_bytes());
        end.extend_from_slice(&start.to_le_bytes());
        end.extend_from_slice(&u16::try_from(comment.len()).unwrap_or(0).to_le_bytes());
        end.extend_from_slice(comment);
        self.put(&end)?;
        Ok(self.out)
    }
}

/// The comment of the archive `f`, from its end record; none when it
/// has none that reads.
pub fn comment<F: Read + Seek>(f: &mut F) -> Option<String> {
    let len = f.seek(SeekFrom::End(0)).ok()?;
    let tail = len.min(22 + MAX_COMMENT as u64);
    f.seek(SeekFrom::End(-i64::try_from(tail).ok()?)).ok()?;
    let mut buf = vec![0u8; usize::try_from(tail).ok()?];
    f.read_exact(&mut buf).ok()?;
    let at = (0..=buf.len().checked_sub(22)?)
        .rev()
        .find(|&i| buf[i..i + 4] == END.to_le_bytes())?;
    let n = usize::from(u16::from_le_bytes([buf[at + 20], buf[at + 21]]));
    let text = buf.get(at + 22..at + 22 + n)?;
    Some(String::from_utf8_lossy(text).into_owned())
}

/// Each entry of the archive `bytes`: its name and its data, read from
/// the central directory and checked by its CRC. For tests and the
/// harness: the host never reads an archive it did not write.
///
/// # Errors
/// What does not read.
pub fn entries(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| -> Result<u16, String> {
        Ok(u16::from_le_bytes(
            bytes
                .get(i..i + 2)
                .ok_or("short")?
                .try_into()
                .map_err(|_| "short")?,
        ))
    };
    let u32_at = |i: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            bytes
                .get(i..i + 4)
                .ok_or("short")?
                .try_into()
                .map_err(|_| "short")?,
        ))
    };
    let end = (0..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&i| u32_at(i).is_ok_and(|s| s == END))
        .ok_or("no end record")?;
    let count = usize::from(u16_at(end + 10)?);
    let mut at = usize::try_from(u32_at(end + 16)?).map_err(|_| "offset")?;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32_at(at)? != CENTRAL {
            return Err("not a central entry".into());
        }
        let crc = u32_at(at + 16)?;
        let size = usize::try_from(u32_at(at + 20)?).map_err(|_| "size")?;
        let name_len = usize::from(u16_at(at + 28)?);
        let skip = usize::from(u16_at(at + 30)?) + usize::from(u16_at(at + 32)?);
        let offset = usize::try_from(u32_at(at + 42)?).map_err(|_| "offset")?;
        let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + name_len).ok_or("name")?)
            .into_owned();
        if u32_at(offset)? != LOCAL {
            return Err(format!("{name}: no local header"));
        }
        let data_at =
            offset + 30 + usize::from(u16_at(offset + 26)?) + usize::from(u16_at(offset + 28)?);
        let data = bytes.get(data_at..data_at + size).ok_or("data")?.to_vec();
        if crc32(&data) != crc {
            return Err(format!("{name}: CRC"));
        }
        out.push((name, data));
        at += 46 + name_len + skip;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_time_are_zips() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        // 2026-11-16 13:15:42 UTC.
        let (t, d) = dos_time(1_794_834_942);
        assert_eq!((t >> 11, (t >> 5) & 63, (t & 31) * 2), (13, 15, 42));
        assert_eq!(((d >> 9) + 1980, (d >> 5) & 15, d & 31), (2026, 11, 16));
    }

    #[test]
    fn an_archive_reads_back_with_its_comment_and_is_the_same_twice() {
        let make = || {
            let mut w = Writer::new(Vec::new(), 1_794_834_942);
            w.add("MANIFEST.txt", b"two files\n").unwrap();
            w.add("log/system.ndjson", &[7u8; 1000]).unwrap();
            w.finish("by: Service Tools").unwrap()
        };
        let bytes = make();
        assert_eq!(bytes, make(), "reproducible");
        let all = entries(&bytes).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[1].0, "log/system.ndjson");
        assert_eq!(all[1].1.len(), 1000);
        let mut c = std::io::Cursor::new(&bytes);
        assert_eq!(comment(&mut c).as_deref(), Some("by: Service Tools"));
        let mut damaged = bytes.clone();
        damaged[100] ^= 1;
        assert!(entries(&damaged).is_err());
    }
}
