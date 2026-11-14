// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A tape volume as a drive sees it: a position among its objects
//! (blocks and filemarks, numbered from 0 at the beginning), the end of
//! data after the last, and room up to its capacity, with an early
//! warning before it. Writing anywhere ends the data there: what came
//! after is gone, as on a tape.
//!
//! The objects' places in the image are found once, when the volume is
//! mounted; a damaged image ends at its last whole object.

use std::io;

use crate::aws::{self, HEADER, Header};

/// The most objects a volume may hold: eight bytes of memory each.
pub const MAX_OBJECTS: usize = 1 << 24;
/// The most bytes of one block.
pub const MAX_BLOCK: usize = 1 << 20;
/// The early warning comes this far before the end, or a hundredth of
/// the capacity when that is less, and never less than a megabyte.
const EARLY_WARNING: u64 = 64 << 20;

/// The bit of an index entry that says the object is a filemark.
const MARK: u64 = 1 << 63;

/// Where a volume's image is kept: a file of the host.
pub trait Store: Send {
    /// Its length.
    fn size(&self) -> io::Result<u64>;
    /// Fill `buf` from `at`.
    fn read_at(&self, buf: &mut [u8], at: u64) -> io::Result<()>;
    /// Write `buf` at `at`.
    fn write_at(&self, buf: &[u8], at: u64) -> io::Result<()>;
    /// Make it `len` bytes long.
    fn truncate(&self, len: u64) -> io::Result<()>;
    /// Make what was written durable.
    fn sync(&self) -> io::Result<()>;
}

impl<S: Store + ?Sized> Store for Box<S> {
    fn size(&self) -> io::Result<u64> {
        (**self).size()
    }
    fn read_at(&self, buf: &mut [u8], at: u64) -> io::Result<()> {
        (**self).read_at(buf, at)
    }
    fn write_at(&self, buf: &[u8], at: u64) -> io::Result<()> {
        (**self).write_at(buf, at)
    }
    fn truncate(&self, len: u64) -> io::Result<()> {
        (**self).truncate(len)
    }
    fn sync(&self) -> io::Result<()> {
        (**self).sync()
    }
}

impl Store for std::fs::File {
    fn size(&self) -> io::Result<u64> {
        self.metadata().map(|m| m.len())
    }
    fn read_at(&self, buf: &mut [u8], at: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::read_exact_at(self, buf, at)
    }
    fn write_at(&self, buf: &[u8], at: u64) -> io::Result<()> {
        std::os::unix::fs::FileExt::write_all_at(self, buf, at)
    }
    fn truncate(&self, len: u64) -> io::Result<()> {
        self.set_len(len)
    }
    fn sync(&self) -> io::Result<()> {
        self.sync_data()
    }
}

/// Why a volume did not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub enum TapeError {
    /// The host's file failed.
    #[error("the volume's file: {0}")]
    Io(#[from] io::Error),
    /// The volume is write-protected.
    #[error("the volume is write-protected")]
    WriteProtected,
    /// The write would pass the end of the volume: nothing was written.
    #[error("the end of the volume")]
    Full,
    /// A block of no bytes or more than the drive takes.
    #[error("a block of {0} bytes")]
    BlockSize(usize),
    /// The volume holds as many objects as it may.
    #[error("the volume holds {MAX_OBJECTS} objects")]
    TooManyObjects,
}

/// What a read found at the position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// A block: its whole length, and the bytes of it copied, which is
    /// less when the caller's room was.
    Block {
        /// The block's length.
        len: usize,
        /// Bytes copied.
        copied: usize,
    },
    /// A filemark: the position is after it.
    Filemark,
    /// The end of data: the position is unchanged.
    EndOfData,
}

/// Where a space or a locate stopped short of what was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// At a filemark: after it going forward, before it going back.
    Filemark,
    /// At the end of data.
    EndOfData,
    /// At the beginning of the volume.
    Beginning,
}

/// A space or locate that stopped early: where, and how many of the
/// count were not done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    /// Where it stopped.
    pub edge: Edge,
    /// What was left of the count.
    pub residue: u64,
}

/// A mounted volume.
pub struct Medium<S> {
    store: S,
    /// Each object's offset in the image, a filemark's with [`MARK`].
    objects: Vec<u64>,
    /// Bytes of the image that hold objects: the end of data.
    end: u64,
    /// The object the drive is before.
    pos: usize,
    capacity: u64,
    protected: bool,
    /// The image had bytes after its last whole object.
    damaged: bool,
}

fn offset(entry: u64) -> u64 {
    entry & !MARK
}

impl<S: Store> Medium<S> {
    /// The volume in `store`, of `capacity` bytes, write-protected or
    /// not, positioned at its beginning.
    pub fn mount(store: S, capacity: u64, protected: bool) -> Result<Self, TapeError> {
        let len = store.size()?;
        let mut objects = Vec::new();
        let mut at = 0u64;
        let mut prev = 0u16;
        let mut damaged = false;
        // Where the last whole object ends.
        let mut good = 0u64;
        'scan: while at + HEADER as u64 <= len {
            if objects.len() == MAX_OBJECTS {
                damaged = true;
                break;
            }
            let start = at;
            let mut first = true;
            loop {
                let Some(h) = header_at(&store, at, len)? else {
                    damaged = true;
                    break 'scan;
                };
                if h.prev != prev {
                    damaged = true;
                    break 'scan;
                }
                if h.is_filemark() {
                    if !first {
                        damaged = true;
                        break 'scan;
                    }
                    objects.push(start | MARK);
                    at += HEADER as u64;
                    good = at;
                    prev = 0;
                    break;
                }
                if first != (h.flags & aws::BEGIN != 0) {
                    damaged = true;
                    break 'scan;
                }
                let next = at + HEADER as u64 + u64::from(h.len);
                if next > len {
                    damaged = true;
                    break 'scan;
                }
                at = next;
                prev = h.len;
                first = false;
                if h.flags & aws::END != 0 {
                    objects.push(start);
                    good = at;
                    break;
                }
            }
        }
        Ok(Self {
            store,
            objects,
            end: good,
            pos: 0,
            capacity,
            protected,
            damaged: damaged || good < len,
        })
    }

    /// The object number the drive is before.
    pub fn position(&self) -> u64 {
        self.pos as u64
    }

    /// Objects on the volume.
    pub fn objects(&self) -> u64 {
        self.objects.len() as u64
    }

    /// Whether the drive is at the beginning.
    pub fn at_beginning(&self) -> bool {
        self.pos == 0
    }

    /// Whether the drive is at the end of data.
    pub fn at_end(&self) -> bool {
        self.pos == self.objects.len()
    }

    /// Bytes of the image in use.
    pub fn used(&self) -> u64 {
        self.end
    }

    /// The volume's capacity.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Whether it is write-protected.
    pub fn protected(&self) -> bool {
        self.protected
    }

    /// Whether the image had bytes past its last whole object.
    pub fn damaged(&self) -> bool {
        self.damaged
    }

    /// Where the early warning starts.
    pub fn warning_at(&self) -> u64 {
        let margin = (self.capacity / 100).clamp(1 << 20, EARLY_WARNING);
        self.capacity.saturating_sub(margin)
    }

    /// Whether the drive is past the early warning.
    pub fn past_warning(&self) -> Result<bool, TapeError> {
        Ok(self.end_of(self.pos)? >= self.warning_at())
    }

    fn is_mark(&self, i: usize) -> bool {
        self.objects[i] & MARK != 0
    }

    /// The image offset where object `i` ends; for `i` 0 the start.
    fn end_of(&self, i: usize) -> Result<u64, TapeError> {
        if i == 0 {
            return Ok(0);
        }
        let entry = self.objects[i - 1];
        if entry & MARK != 0 {
            return Ok(offset(entry) + HEADER as u64);
        }
        let (_, end, _) = self.walk(offset(entry), &mut [])?;
        Ok(end)
    }

    /// The length of the last chunk before object `i`.
    fn prev_len(&self, i: usize) -> Result<u16, TapeError> {
        if i == 0 || self.is_mark(i - 1) {
            return Ok(0);
        }
        let (_, _, last) = self.walk(offset(self.objects[i - 1]), &mut [])?;
        Ok(last)
    }

    /// Read the block at `at` into `buf`: its length, where it ends,
    /// and its last chunk's length.
    fn walk(&self, mut at: u64, buf: &mut [u8]) -> Result<(usize, u64, u16), TapeError> {
        let mut len = 0usize;
        let mut copied = 0usize;
        loop {
            let Some(h) = header_at(&self.store, at, u64::MAX)? else {
                return Err(TapeError::Io(io::Error::other("a block's header changed")));
            };
            let n = usize::from(h.len);
            let data = at + HEADER as u64;
            if copied < buf.len() {
                let take = n.min(buf.len() - copied);
                self.store.read_at(&mut buf[copied..copied + take], data)?;
                copied += take;
            }
            len = len.saturating_add(n);
            at = data + n as u64;
            if h.flags & aws::END != 0 {
                return Ok((len, at, h.len));
            }
        }
    }

    /// Read the object at the position into `buf`.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<Read, TapeError> {
        if self.at_end() {
            return Ok(Read::EndOfData);
        }
        if self.is_mark(self.pos) {
            self.pos += 1;
            return Ok(Read::Filemark);
        }
        let (len, _, _) = self.walk(offset(self.objects[self.pos]), buf)?;
        self.pos += 1;
        Ok(Read::Block {
            len,
            copied: len.min(buf.len()),
        })
    }

    /// Cut the data at the position: what was after it is gone.
    fn cut(&mut self) -> Result<u64, TapeError> {
        if self.protected {
            return Err(TapeError::WriteProtected);
        }
        let at = self.end_of(self.pos)?;
        Ok(at)
    }

    fn append(&mut self, at: u64, bytes: &[u8], marks: &[u64]) -> Result<(), TapeError> {
        if self.objects.len() + marks.len() > MAX_OBJECTS {
            return Err(TapeError::TooManyObjects);
        }
        self.store.write_at(bytes, at)?;
        let end = at + bytes.len() as u64;
        self.store.truncate(end)?;
        self.objects.truncate(self.pos);
        self.objects.extend_from_slice(marks);
        self.pos = self.objects.len();
        self.end = end;
        Ok(())
    }

    /// Write a block at the position. Whether the drive is now past the
    /// early warning. A block that would pass the capacity is not
    /// written.
    pub fn write(&mut self, data: &[u8]) -> Result<bool, TapeError> {
        if data.is_empty() || data.len() > MAX_BLOCK {
            return Err(TapeError::BlockSize(data.len()));
        }
        let at = self.cut()?;
        if at + aws::block_bytes(data.len()) > self.capacity {
            return Err(TapeError::Full);
        }
        let prev = self.prev_len(self.pos)?;
        let mut bytes = Vec::with_capacity(data.len() + 6 * (data.len() / aws::MAX_CHUNK + 1));
        let mut rest = data;
        for h in Header::for_block(data.len(), prev) {
            bytes.extend_from_slice(&h.encode());
            let (chunk, after) = rest.split_at(usize::from(h.len));
            bytes.extend_from_slice(chunk);
            rest = after;
        }
        self.append(at, &bytes, &[at])?;
        Ok(self.end >= self.warning_at())
    }

    /// Write `count` filemarks at the position. Whether the drive is
    /// now past the early warning.
    pub fn write_filemarks(&mut self, count: u32) -> Result<bool, TapeError> {
        let at = self.cut()?;
        if count == 0 {
            return Ok(at >= self.warning_at());
        }
        let n = u64::from(count);
        if at + n * HEADER as u64 > self.capacity {
            return Err(TapeError::Full);
        }
        let mut prev = self.prev_len(self.pos)?;
        let mut bytes = Vec::new();
        let mut marks = Vec::new();
        for i in 0..n {
            bytes.extend_from_slice(&Header::filemark(prev).encode());
            marks.push((at + i * HEADER as u64) | MARK);
            prev = 0;
        }
        self.append(at, &bytes, &marks)?;
        Ok(self.end >= self.warning_at())
    }

    /// Erase from the position to the end: the end of data is here.
    pub fn erase(&mut self) -> Result<(), TapeError> {
        let at = self.cut()?;
        self.store.truncate(at)?;
        self.objects.truncate(self.pos);
        self.end = at;
        Ok(())
    }

    /// Make every write so far durable.
    pub fn sync(&self) -> Result<(), TapeError> {
        Ok(self.store.sync()?)
    }

    /// To the beginning.
    pub fn rewind(&mut self) {
        self.pos = 0;
    }

    /// To the end of data.
    pub fn space_to_end(&mut self) {
        self.pos = self.objects.len();
    }

    /// Before object `object`; beyond the data, at its end.
    pub fn locate(&mut self, object: u64) -> Result<(), Stop> {
        match usize::try_from(object) {
            Ok(i) if i <= self.objects.len() => {
                self.pos = i;
                Ok(())
            }
            _ => {
                self.pos = self.objects.len();
                Err(Stop {
                    edge: Edge::EndOfData,
                    residue: 0,
                })
            }
        }
    }

    /// Space over `count` blocks, back when negative. A filemark stops
    /// it: forward the drive is after the mark, back before it.
    pub fn space_blocks(&mut self, count: i64) -> Result<(), Stop> {
        let n = count.unsigned_abs();
        for done in 0..n {
            let stop = |edge| Stop {
                edge,
                residue: n - done,
            };
            if count > 0 {
                if self.at_end() {
                    return Err(stop(Edge::EndOfData));
                }
                let mark = self.is_mark(self.pos);
                self.pos += 1;
                if mark {
                    return Err(stop(Edge::Filemark));
                }
            } else {
                if self.pos == 0 {
                    return Err(stop(Edge::Beginning));
                }
                self.pos -= 1;
                if self.is_mark(self.pos) {
                    return Err(stop(Edge::Filemark));
                }
            }
        }
        Ok(())
    }

    /// Space over `count` filemarks, back when negative: forward to
    /// after the last, back to before it.
    pub fn space_filemarks(&mut self, count: i64) -> Result<(), Stop> {
        let n = count.unsigned_abs();
        let mut done = 0;
        while done < n {
            if count > 0 {
                if self.at_end() {
                    return Err(Stop {
                        edge: Edge::EndOfData,
                        residue: n - done,
                    });
                }
                if self.is_mark(self.pos) {
                    done += 1;
                }
                self.pos += 1;
            } else {
                if self.pos == 0 {
                    return Err(Stop {
                        edge: Edge::Beginning,
                        residue: n - done,
                    });
                }
                self.pos -= 1;
                if self.is_mark(self.pos) {
                    done += 1;
                }
            }
        }
        Ok(())
    }

    /// The store, for the caller to give back.
    pub fn into_store(self) -> S {
        self.store
    }
}

/// The header at `at`, when there are six bytes there that are one.
fn header_at<S: Store>(store: &S, at: u64, len: u64) -> Result<Option<Header>, TapeError> {
    if at.saturating_add(HEADER as u64) > len {
        return Ok(None);
    }
    let mut b = [0u8; HEADER];
    store.read_at(&mut b, at)?;
    Ok(Header::decode(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// An image in memory, shared so a test can look at it.
    #[derive(Clone, Default)]
    pub struct Mem(pub Arc<Mutex<Vec<u8>>>);

    impl Store for Mem {
        fn size(&self) -> io::Result<u64> {
            Ok(self.0.lock().unwrap().len() as u64)
        }
        fn read_at(&self, buf: &mut [u8], at: u64) -> io::Result<()> {
            let v = self.0.lock().unwrap();
            let at = usize::try_from(at).unwrap();
            let src = v
                .get(at..at + buf.len())
                .ok_or_else(|| io::Error::other("past the end"))?;
            buf.copy_from_slice(src);
            Ok(())
        }
        fn write_at(&self, buf: &[u8], at: u64) -> io::Result<()> {
            let mut v = self.0.lock().unwrap();
            let at = usize::try_from(at).unwrap();
            if v.len() < at + buf.len() {
                v.resize(at + buf.len(), 0);
            }
            v[at..at + buf.len()].copy_from_slice(buf);
            Ok(())
        }
        fn truncate(&self, len: u64) -> io::Result<()> {
            self.0
                .lock()
                .unwrap()
                .resize(usize::try_from(len).unwrap(), 0);
            Ok(())
        }
        fn sync(&self) -> io::Result<()> {
            Ok(())
        }
    }

    fn medium(capacity: u64) -> Medium<Mem> {
        Medium::mount(Mem::default(), capacity, false).unwrap()
    }

    fn read(m: &mut Medium<Mem>) -> (Read, Vec<u8>) {
        let mut buf = vec![0u8; 200_000];
        let r = m.read(&mut buf).unwrap();
        let n = match r {
            Read::Block { copied, .. } => copied,
            _ => 0,
        };
        buf.truncate(n);
        (r, buf)
    }

    #[test]
    fn blocks_and_filemarks_are_read_back_after_a_mount() {
        let mut m = medium(1 << 30);
        m.write(b"label").unwrap();
        m.write_filemarks(1).unwrap();
        let big: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
        m.write(&big).unwrap();
        m.write_filemarks(2).unwrap();
        assert_eq!(m.objects(), 5);
        let image = m.into_store();
        let mut m = Medium::mount(image.clone(), 1 << 30, false).unwrap();
        assert!(!m.damaged());
        assert_eq!(m.objects(), 5);
        assert_eq!(
            read(&mut m),
            (Read::Block { len: 5, copied: 5 }, b"label".to_vec())
        );
        assert_eq!(read(&mut m).0, Read::Filemark);
        assert_eq!(
            read(&mut m),
            (
                Read::Block {
                    len: 150_000,
                    copied: 150_000
                },
                big
            )
        );
        assert_eq!(read(&mut m).0, Read::Filemark);
        assert_eq!(read(&mut m).0, Read::Filemark);
        assert_eq!(read(&mut m).0, Read::EndOfData);
        assert_eq!(m.position(), 5);
        // A short room: the block's length and what fitted.
        m.rewind();
        let mut two = [0u8; 2];
        assert_eq!(m.read(&mut two).unwrap(), Read::Block { len: 5, copied: 2 });
        assert_eq!(&two, b"la");
        // The format is AWSTAPE's: the first header is the label's.
        let bytes = image.0.lock().unwrap().clone();
        assert_eq!(&bytes[..6], &[5, 0, 0, 0, 0xA0, 0]);
        assert_eq!(&bytes[11..17], &[0, 0, 5, 0, 0x40, 0]);
    }

    #[test]
    fn writing_in_the_middle_ends_the_data_there() {
        let mut m = medium(1 << 30);
        for b in [b"one", b"two", b"six"] {
            m.write(b).unwrap();
        }
        m.locate(1).unwrap();
        m.write(b"TWO!").unwrap();
        assert_eq!(m.objects(), 2);
        assert!(m.at_end());
        m.rewind();
        assert_eq!(read(&mut m).1, b"one");
        assert_eq!(read(&mut m).1, b"TWO!");
        assert_eq!(read(&mut m).0, Read::EndOfData);
        let used = m.used();
        assert_eq!(used, 3 + 6 + 4 + 6);
        assert_eq!(m.into_store().0.lock().unwrap().len() as u64, used);
    }

    #[test]
    fn spacing_stops_at_filemarks_and_the_ends() {
        let mut m = medium(1 << 30);
        m.write(b"a").unwrap();
        m.write(b"b").unwrap();
        m.write_filemarks(1).unwrap();
        m.write(b"c").unwrap();
        m.write_filemarks(1).unwrap();
        m.rewind();
        // Forward over blocks: the mark stops it, after the mark.
        assert_eq!(
            m.space_blocks(5),
            Err(Stop {
                edge: Edge::Filemark,
                residue: 3
            })
        );
        assert_eq!(m.position(), 3);
        // Back over blocks: the mark stops it, before the mark.
        assert_eq!(
            m.space_blocks(-1),
            Err(Stop {
                edge: Edge::Filemark,
                residue: 1
            })
        );
        assert_eq!(m.position(), 2);
        assert_eq!(
            m.space_blocks(-5),
            Err(Stop {
                edge: Edge::Beginning,
                residue: 3
            })
        );
        // Filemarks: forward to after the second.
        m.space_filemarks(2).unwrap();
        assert!(m.at_end());
        assert_eq!(
            m.space_filemarks(1),
            Err(Stop {
                edge: Edge::EndOfData,
                residue: 1
            })
        );
        // Back one: before the last mark.
        m.space_filemarks(-1).unwrap();
        assert_eq!(m.position(), 4);
        assert_eq!(read(&mut m).0, Read::Filemark);
        m.space_to_end();
        assert_eq!(m.position(), 5);
        assert!(m.locate(9).is_err());
        assert_eq!(m.position(), 5);
    }

    #[test]
    fn the_early_warning_comes_before_the_end_and_the_end_refuses() {
        let cap = 4 << 20;
        let mut m = medium(cap);
        let block = vec![7u8; 512 * 1024];
        let mut warned = 0;
        let mut written = 0;
        loop {
            match m.write(&block) {
                Ok(w) => {
                    written += 1;
                    if w {
                        warned += 1;
                    }
                }
                Err(TapeError::Full) => break,
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(written, 7, "a 4 MiB volume takes seven 512 KiB blocks");
        assert!(warned >= 1);
        assert!(m.used() <= cap);
        // Nothing of the refused block was written.
        assert_eq!(m.objects(), 7);
    }

    #[test]
    fn a_protected_volume_is_not_written() {
        let mut m = Medium::mount(Mem::default(), 1 << 20, true).unwrap();
        assert!(matches!(m.write(b"x"), Err(TapeError::WriteProtected)));
        assert!(matches!(
            m.write_filemarks(1),
            Err(TapeError::WriteProtected)
        ));
        assert!(matches!(m.erase(), Err(TapeError::WriteProtected)));
        assert!(matches!(m.write(&[]), Err(TapeError::BlockSize(0))));
    }

    #[test]
    fn a_damaged_tail_ends_the_data_at_the_last_whole_object() {
        let mut m = medium(1 << 30);
        m.write(b"good").unwrap();
        m.write_filemarks(1).unwrap();
        m.write(b"lost").unwrap();
        let image = m.into_store();
        {
            let mut v = image.0.lock().unwrap();
            let n = v.len();
            v.truncate(n - 2); // the last block cut short
        }
        let mut m = Medium::mount(image.clone(), 1 << 30, false).unwrap();
        assert!(m.damaged());
        assert_eq!(m.objects(), 2);
        assert_eq!(m.used(), 4 + 6 + 6);
        m.space_to_end();
        m.write(b"new").unwrap();
        m.rewind();
        assert_eq!(read(&mut m).1, b"good");
        assert_eq!(read(&mut m).0, Read::Filemark);
        assert_eq!(read(&mut m).1, b"new");
        // Garbage where a header should be.
        let junk = Mem(Arc::new(Mutex::new(vec![0xFF; 64])));
        let m = Medium::mount(junk, 1 << 20, false).unwrap();
        assert!(m.damaged());
        assert_eq!(m.objects(), 0);
        assert_eq!(m.used(), 0);
    }
}
