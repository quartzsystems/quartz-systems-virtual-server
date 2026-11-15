// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's dump (`docs/DUMPS.md`): what the host takes of a
//! partition from outside, its processors stopped. The file is an ELF
//! core of x86-64, the form every tool that reads cores takes:
//!
//! - one `PT_NOTE` segment: a `CORE`/`NT_PRSTATUS` note for each
//!   processor with its general registers, a `QSVS` note of each
//!   processor's special registers, and the `QSVS` header record;
//! - one `PT_LOAD` segment for each region of guest memory, its
//!   physical address as both addresses, its bytes at a page-aligned
//!   offset. A page of zeroes is not written: the file has a hole there
//!   and takes no space for it.
//!
//! - [`write`]: the writer, over a [`Source`] of guest memory.
//! - [`read_header`]: the header record of a dump, for a list.
//! - [`limits`]: which dumps go when a partition has too many.

pub mod limits;
mod note;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::FileExt;

pub use note::{Cpu, Header};

/// A page: what is written or left a hole.
pub const PAGE: u64 = 4096;
/// The ELF header's size.
const EHDR: u64 = 64;
/// A program header's size.
const PHDR: u64 = 56;
/// `PT_LOAD`, `PT_NOTE`.
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
/// `ET_CORE`, `EM_X86_64`.
const ET_CORE: u16 = 4;
const EM_X86_64: u16 = 62;
/// The most bytes of notes read back: a few hundred processors.
const MAX_NOTES: u64 = 4 << 20;
/// The suffix of a dump's file.
pub const SUFFIX: &str = ".core";
/// The suffix of a dump being written; it is renamed when whole.
pub const PARTIAL: &str = ".partial";
/// The bytes of memory read at once.
const CHUNK: u64 = 2 << 20;

/// Guest memory, as the dump reads it.
pub trait Source {
    /// Each region: its guest-physical address and its bytes.
    fn regions(&self) -> Vec<(u64, u64)>;
    /// Read `buf.len()` bytes at the guest-physical `address`.
    fn read(&self, address: u64, buf: &mut [u8]) -> io::Result<()>;
}

/// What a written dump is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Written {
    /// Its size: what a reader sees.
    pub bytes: u64,
    /// The bytes of memory written: the rest were pages of zeroes.
    pub data: u64,
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn round_up(v: u64, to: u64) -> u64 {
    v.div_ceil(to) * to
}

/// The ELF header and program headers of a dump with `notes` bytes of
/// notes and `regions`, and each region's offset in the file.
fn headers(notes: u64, regions: &[(u64, u64)]) -> (Vec<u8>, Vec<u64>) {
    let count = u16::try_from(regions.len() + 1).unwrap_or(u16::MAX);
    let mut out = Vec::new();
    out.extend_from_slice(&[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
    out.extend_from_slice(&[0; 8]);
    put16(&mut out, ET_CORE);
    put16(&mut out, EM_X86_64);
    put32(&mut out, 1);
    put64(&mut out, 0); // entry
    put64(&mut out, EHDR); // program headers
    put64(&mut out, 0); // no section headers
    put32(&mut out, 0);
    put16(&mut out, 64);
    put16(&mut out, 56);
    put16(&mut out, count);
    put16(&mut out, 64);
    put16(&mut out, 0);
    put16(&mut out, 0);
    let notes_at = EHDR + PHDR * u64::from(count);
    // PT_NOTE.
    put32(&mut out, PT_NOTE);
    put32(&mut out, 0);
    put64(&mut out, notes_at);
    put64(&mut out, 0);
    put64(&mut out, 0);
    put64(&mut out, notes);
    put64(&mut out, 0);
    put64(&mut out, 4);
    let mut at = round_up(notes_at + notes, PAGE);
    let mut offsets = Vec::with_capacity(regions.len());
    for (start, len) in regions {
        put32(&mut out, PT_LOAD);
        put32(&mut out, 7); // read, write, execute
        put64(&mut out, at);
        put64(&mut out, *start);
        put64(&mut out, *start);
        put64(&mut out, *len);
        put64(&mut out, *len);
        put64(&mut out, PAGE);
        offsets.push(at);
        at = round_up(at + len, PAGE);
    }
    (out, offsets)
}

/// Write a dump of `source` with the processors `cpus` and the record
/// `header` to `out`, which is empty. `progress` is told each tenth of
/// the memory, in percent. Pages of zeroes are left holes.
///
/// # Errors
/// A write fails: the file is then not a dump, and its writer removes it.
pub fn write(
    out: &File,
    header: &Header,
    cpus: &[Cpu],
    source: &dyn Source,
    progress: &mut dyn FnMut(u8),
) -> io::Result<Written> {
    let notes = note::all(header, cpus);
    let regions = source.regions();
    let (head, offsets) = headers(notes.len() as u64, &regions);
    out.write_all_at(&head, 0)?;
    out.write_all_at(&notes, head.len() as u64)?;
    let total: u64 = regions.iter().map(|(_, len)| len).sum();
    let mut done = 0u64;
    let mut told = 0u8;
    let mut data = 0u64;
    let mut end = head.len() as u64 + notes.len() as u64;
    let mut buf = vec![0u8; usize::try_from(CHUNK).unwrap_or(1 << 21)];
    for ((start, len), at) in regions.iter().zip(&offsets) {
        let mut off = 0u64;
        while off < *len {
            let n = CHUNK.min(len - off);
            let slice = &mut buf[..usize::try_from(n).unwrap_or(0)];
            source.read(start + off, slice)?;
            for (i, page) in slice
                .chunks(usize::try_from(PAGE).unwrap_or(4096))
                .enumerate()
            {
                if page.iter().any(|b| *b != 0) {
                    let place = at + off + i as u64 * PAGE;
                    out.write_all_at(page, place)?;
                    data += page.len() as u64;
                }
            }
            off += n;
            done += n;
            if let Some(percent) = (done * 100).checked_div(total) {
                #[allow(clippy::cast_possible_truncation)] // at most 100
                let percent = percent as u8;
                if percent / 10 > told / 10 {
                    told = percent;
                    progress(percent);
                }
            }
        }
        end = end.max(at + len);
    }
    out.set_len(end)?;
    out.sync_all()?;
    Ok(Written { bytes: end, data })
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// The header record of the dump in `file`. Everything read is checked:
/// a file that is not a dump of this form is refused.
///
/// # Errors
/// It cannot be read, or is not a dump.
pub fn read_header(file: &mut File) -> io::Result<Header> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_string());
    let mut ehdr = [0u8; 64];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut ehdr)?;
    if ehdr[..6] != [0x7F, b'E', b'L', b'F', 2, 1] || u16_at(&ehdr, 16) != Some(ET_CORE) {
        return Err(bad("not an ELF core of 64 bits"));
    }
    let phoff = u64_at(&ehdr, 32).ok_or_else(|| bad("the header"))?;
    let phnum = u16_at(&ehdr, 56).ok_or_else(|| bad("the header"))?;
    for i in 0..u64::from(phnum) {
        let mut ph = [0u8; 56];
        file.seek(SeekFrom::Start(phoff + i * PHDR))?;
        file.read_exact(&mut ph)?;
        if u32_at(&ph, 0) != Some(PT_NOTE) {
            continue;
        }
        let offset = u64_at(&ph, 8).ok_or_else(|| bad("the notes"))?;
        let size = u64_at(&ph, 32).ok_or_else(|| bad("the notes"))?;
        if size > MAX_NOTES {
            return Err(bad("the notes are too large"));
        }
        let mut notes = vec![0u8; usize::try_from(size).map_err(|_| bad("the notes"))?];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut notes)?;
        return note::header_of(&notes).ok_or_else(|| bad("no header record"));
    }
    Err(bad("no notes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    struct Mem(Vec<(u64, Vec<u8>)>);

    impl Source for Mem {
        fn regions(&self) -> Vec<(u64, u64)> {
            self.0.iter().map(|(s, b)| (*s, b.len() as u64)).collect()
        }
        fn read(&self, address: u64, buf: &mut [u8]) -> io::Result<()> {
            for (s, b) in &self.0 {
                if address >= *s && address + buf.len() as u64 <= s + b.len() as u64 {
                    let at = usize::try_from(address - s).unwrap();
                    buf.copy_from_slice(&b[at..at + buf.len()]);
                    return Ok(());
                }
            }
            Err(io::Error::other("outside"))
        }
    }

    pub(crate) fn header() -> Header {
        Header {
            partition: "PROD".into(),
            id: 3,
            serial: "K2M4103".into(),
            code: "B1001234".into(),
            text: "LIC failed".into(),
            time: 1_794_000_000,
            release: "V1R0M0".into(),
            reason: "function 34".into(),
        }
    }

    #[test]
    fn a_dump_is_an_elf_core_with_holes_for_zeroes_and_its_header() {
        let path = std::env::temp_dir().join(format!("dump-{}.core", std::process::id()));
        let mut low = vec![0u8; 64 << 20];
        low[0x1234] = 0xAB;
        low[(32 << 20) + 7] = 0xCD;
        let high = vec![0u8; 16 << 20];
        let mem = Mem(vec![(0, low), (0x1_0000_0000, high)]);
        let file = File::create(&path).unwrap();
        let cpus = [
            Cpu::default(),
            Cpu {
                rip: 0xFFFF_FFFF_8100_0000,
                ..Cpu::default()
            },
        ];
        let mut told = Vec::new();
        let w = write(&file, &header(), &cpus, &mem, &mut |p| told.push(p)).unwrap();
        assert_eq!(w.data, 2 * PAGE);
        assert_eq!(told.len(), 10);
        assert_eq!(*told.last().unwrap(), 100);
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), w.bytes);
        assert!(w.bytes > 80 << 20);
        // Far below the memory on disk: the zero pages are holes.
        assert!(meta.blocks() * 512 < 1 << 20, "{} blocks", meta.blocks());
        let mut back = File::open(&path).unwrap();
        assert_eq!(read_header(&mut back).unwrap(), header());
        // Each byte is where its region's program header says.
        let notes = note::all(&header(), &cpus);
        let (_, offsets) = headers(notes.len() as u64, &mem.regions());
        let mut b = [0u8; 1];
        back.read_exact_at(&mut b, offsets[0] + 0x1234).unwrap();
        assert_eq!(b, [0xAB]);
        back.read_exact_at(&mut b, offsets[0] + (32 << 20) + 7)
            .unwrap();
        assert_eq!(b, [0xCD]);
        assert_eq!(offsets[1] % PAGE, 0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_file_that_is_not_a_dump_is_refused() {
        let path = std::env::temp_dir().join(format!("dump-bad-{}", std::process::id()));
        std::fs::write(
            &path,
            b"not an elf at all, and long enough for a header ..........",
        )
        .unwrap();
        assert!(read_header(&mut File::open(&path).unwrap()).is_err());
        let mut fake = vec![0x7F, b'E', b'L', b'F', 2, 1, 1, 0];
        fake.resize(64, 0);
        fake[16] = 4;
        fake[32] = 64;
        fake[56] = 1;
        let mut ph = vec![0u8; 56];
        ph[0] = 4;
        ph[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        fake.extend(ph);
        std::fs::write(&path, &fake).unwrap();
        assert!(read_header(&mut File::open(&path).unwrap()).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
