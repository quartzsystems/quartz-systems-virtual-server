// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One partition of the load-source disk as a device of its own. No
//! read or write leaves the partition, whatever is asked.

use std::io::{self, Read, Seek, SeekFrom, Write};

/// `len` bytes of a device from `start`, addressed from zero.
pub struct Part<D> {
    dev: D,
    start: u64,
    len: u64,
    pos: u64,
}

impl<D> Part<D> {
    /// The partition `[start, start + len)` of `dev`.
    pub fn new(dev: D, start: u64, len: u64) -> Self {
        Self {
            dev,
            start,
            len,
            pos: 0,
        }
    }

    /// Its length.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether it has no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Give the device back.
    pub fn into_inner(self) -> D {
        self.dev
    }

    fn room(&self, want: usize) -> usize {
        usize::try_from(self.len.saturating_sub(self.pos))
            .unwrap_or(usize::MAX)
            .min(want)
    }
}

impl<D: Read + Seek> Read for Part<D> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.room(buf.len());
        if n == 0 {
            return Ok(0);
        }
        self.dev.seek(SeekFrom::Start(self.start + self.pos))?;
        let got = self.dev.read(&mut buf[..n])?;
        self.pos += got as u64;
        Ok(got)
    }
}

impl<D: Write + Seek> Write for Part<D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.room(buf.len());
        if n == 0 && !buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "past the partition's end",
            ));
        }
        self.dev.seek(SeekFrom::Start(self.start + self.pos))?;
        let put = self.dev.write(&buf[..n])?;
        self.pos += put as u64;
        Ok(put)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.dev.flush()
    }
}

impl<D: Seek> Seek for Part<D> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(n) => self.len.checked_add_signed(n),
            SeekFrom::Current(n) => self.pos.checked_add_signed(n),
        };
        // A position beyond the end is allowed, as on a file: a read
        // there is empty and a write fails.
        self.pos = target.ok_or_else(|| io::Error::other("seek before the partition"))?;
        Ok(self.pos)
    }
}

impl<D: abrec::Device> abrec::Device for Part<D> {
    fn sync(&mut self) -> io::Result<()> {
        self.dev.sync()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn nothing_is_read_or_written_outside_the_partition() {
        let mut disk = Cursor::new((0..=255u8).collect::<Vec<u8>>());
        let mut part = Part::new(&mut disk, 100, 50);
        let mut all = Vec::new();
        part.read_to_end(&mut all).unwrap();
        assert_eq!(all, (100..150u8).collect::<Vec<u8>>());
        part.seek(SeekFrom::Start(48)).unwrap();
        assert_eq!(part.write(&[1, 2, 3, 4]).unwrap(), 2);
        assert!(part.write(&[3]).is_err());
        assert_eq!(part.seek(SeekFrom::End(-1)).unwrap(), 49);
        assert!(part.seek(SeekFrom::Current(-60)).is_err());
        part.seek(SeekFrom::Start(u64::MAX)).unwrap();
        assert_eq!(part.read(&mut [0u8; 4]).unwrap(), 0);
        let bytes = disk.into_inner();
        assert_eq!(bytes[148..152], [1, 2, 150, 151]);
    }
}
