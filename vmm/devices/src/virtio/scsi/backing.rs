// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a LUN's blocks are kept in: a file or a block device of the
//! host, opened for direct I/O. Nothing the guest wrote waits in the
//! host's page cache, and a guest's flush is a flush of the host's
//! device: the guest's own crash consistency rests on that.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

use rustix::fs::{FallocateFlags, fallocate};

/// Direct I/O wants its buffers on this boundary.
pub const ALIGNMENT: usize = 4096;

/// A buffer on the boundary direct I/O wants, of a fixed length.
pub struct Bounce {
    raw: Vec<u8>,
    start: usize,
    len: usize,
}

impl Bounce {
    /// A buffer of `len` bytes.
    pub fn new(len: usize) -> Self {
        let raw = vec![0u8; len + ALIGNMENT];
        let start = raw.as_ptr().align_offset(ALIGNMENT);
        Self { raw, start, len }
    }

    /// Its length.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether it has no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The buffer.
    pub fn bytes(&mut self) -> &mut [u8] {
        &mut self.raw[self.start..self.start + self.len]
    }
}

/// A LUN's backing store.
pub trait Backing: Send + Sync {
    /// Its length in bytes.
    fn bytes(&self) -> u64;
    /// Bytes of a block: every offset and length is a multiple.
    fn block_bytes(&self) -> u32;
    /// Fill `buf` from `offset`.
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
    /// Write `buf` at `offset`.
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()>;
    /// Everything written is on stable storage when this returns.
    fn flush(&self) -> io::Result<()>;
    /// Whether [`Backing::discard`] does anything.
    fn can_discard(&self) -> bool;
    /// Whether discarded blocks read back as zeros.
    fn discard_zeroes(&self) -> bool;
    /// Give the blocks of a range back.
    fn discard(&self, offset: u64, bytes: u64) -> io::Result<()>;
    /// Make a range read as zeros.
    fn write_zeroes(&self, offset: u64, bytes: u64) -> io::Result<()>;
}

/// A file or block device of the host.
pub struct HostStore {
    file: File,
    bytes: u64,
    block_bytes: u32,
    block_device: bool,
    discards: bool,
}

/// `O_DIRECT`.
const O_DIRECT: i32 = 0o40000;
/// `O_EXCL`: without `O_CREAT`, on a block device, an exclusive open.
const O_EXCL: i32 = 0o200;

fn queue_value(dev: u64, name: &str) -> Option<u64> {
    let (major, minor) = (rustix::fs::major(dev), rustix::fs::minor(dev));
    let node = format!("/sys/dev/block/{major}:{minor}");
    // A partition's queue is its disk's.
    [
        format!("{node}/queue/{name}"),
        format!("{node}/../queue/{name}"),
    ]
    .iter()
    .find_map(|p| std::fs::read_to_string(p).ok())
    .and_then(|s| s.trim().parse().ok())
}

impl HostStore {
    /// Open `path` for direct I/O, reading and writing. A block device
    /// is opened exclusively: one the host has mounted, or another
    /// partition has, is refused.
    pub fn open(path: &Path) -> io::Result<Self> {
        let exclusive = if std::fs::metadata(path)?.file_type().is_block_device() {
            O_EXCL
        } else {
            0
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_DIRECT | exclusive)
            .open(path)?;
        let meta = file.metadata()?;
        let block_device = meta.file_type().is_block_device();
        if !block_device && !meta.file_type().is_file() {
            return Err(io::Error::other("neither a file nor a block device"));
        }
        let (bytes, block_bytes, discards) = if block_device {
            let sectors = queue_value(meta.rdev(), "../size")
                .ok_or_else(|| io::Error::other("the device's size"))?;
            let block = queue_value(meta.rdev(), "logical_block_size").unwrap_or(512);
            let discard = queue_value(meta.rdev(), "discard_max_bytes").unwrap_or(0);
            (sectors * 512, block, discard > 0)
        } else {
            (meta.len(), 512, true)
        };
        let block_bytes = u32::try_from(block_bytes)
            .ok()
            .filter(|b| b.is_power_of_two() && (512..=4096).contains(b))
            .ok_or_else(|| io::Error::other("the block size"))?;
        let bytes = bytes - bytes % u64::from(block_bytes);
        if bytes == 0 {
            return Err(io::Error::other("no blocks"));
        }
        Ok(Self {
            file,
            bytes,
            block_bytes,
            block_device,
            discards,
        })
    }
}

impl Backing for HostStore {
    fn bytes(&self) -> u64 {
        self.bytes
    }

    fn block_bytes(&self) -> u32 {
        self.block_bytes
    }

    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.file.read_exact_at(buf, offset)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        self.file.write_all_at(buf, offset)
    }

    fn flush(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    fn can_discard(&self) -> bool {
        self.discards
    }

    fn discard_zeroes(&self) -> bool {
        // A hole in a file reads as zeros; a device says nothing of
        // what it gives back.
        !self.block_device
    }

    fn discard(&self, offset: u64, bytes: u64) -> io::Result<()> {
        if self.block_device {
            crate::sys::discard(&self.file, offset, bytes)
        } else {
            let flags = FallocateFlags::PUNCH_HOLE | FallocateFlags::KEEP_SIZE;
            Ok(fallocate(&self.file, flags, offset, bytes)?)
        }
    }

    fn write_zeroes(&self, offset: u64, bytes: u64) -> io::Result<()> {
        if self.block_device {
            crate::sys::zero_out(&self.file, offset, bytes)
        } else {
            let flags = FallocateFlags::ZERO_RANGE | FallocateFlags::KEEP_SIZE;
            Ok(fallocate(&self.file, flags, offset, bytes)?)
        }
    }
}

/// A backing store in memory, for the tests and the fuzzer: it counts
/// what was asked of it.
pub mod memory {
    use super::{Backing, io};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Blocks in memory.
    pub struct MemoryStore {
        /// The bytes.
        pub data: Mutex<Vec<u8>>,
        /// Flushes asked for.
        pub flushes: AtomicUsize,
        /// Ranges discarded.
        pub discards: Mutex<Vec<(u64, u64)>>,
        /// Whether it takes discards.
        pub discardable: bool,
    }

    impl MemoryStore {
        /// A store of `bytes` zeros.
        pub fn new(bytes: usize, discardable: bool) -> Self {
            Self {
                data: Mutex::new(vec![0; bytes]),
                flushes: AtomicUsize::new(0),
                discards: Mutex::new(Vec::new()),
                discardable,
            }
        }

        fn range(&self, offset: u64, bytes: u64) -> io::Result<(usize, usize)> {
            let len = self.data.lock().map_or(0, |d| d.len());
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            let end = usize::try_from(bytes)
                .ok()
                .and_then(|b| start.checked_add(b))
                .filter(|end| *end <= len)
                .ok_or_else(|| io::Error::other("beyond the store"))?;
            Ok((start, end))
        }
    }

    impl Backing for MemoryStore {
        fn bytes(&self) -> u64 {
            self.data.lock().map_or(0, |d| d.len() as u64)
        }

        fn block_bytes(&self) -> u32 {
            512
        }

        fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
            let (start, end) = self.range(offset, buf.len() as u64)?;
            let data = self.data.lock().map_err(|_| io::Error::other("poisoned"))?;
            buf.copy_from_slice(&data[start..end]);
            Ok(())
        }

        fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
            let (start, end) = self.range(offset, buf.len() as u64)?;
            let mut data = self.data.lock().map_err(|_| io::Error::other("poisoned"))?;
            data[start..end].copy_from_slice(buf);
            Ok(())
        }

        fn flush(&self) -> io::Result<()> {
            self.flushes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn can_discard(&self) -> bool {
            self.discardable
        }

        fn discard_zeroes(&self) -> bool {
            true
        }

        fn discard(&self, offset: u64, bytes: u64) -> io::Result<()> {
            self.write_zeroes(offset, bytes)?;
            if let Ok(mut discards) = self.discards.lock() {
                discards.push((offset, bytes));
            }
            Ok(())
        }

        fn write_zeroes(&self, offset: u64, bytes: u64) -> io::Result<()> {
            let (start, end) = self.range(offset, bytes)?;
            let mut data = self.data.lock().map_err(|_| io::Error::other("poisoned"))?;
            data[start..end].fill(0);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bounce_buffer_is_aligned_for_direct_io() {
        for len in [512, 4096, 1 << 20] {
            let mut bounce = Bounce::new(len);
            assert_eq!(bounce.len(), len);
            assert_eq!(bounce.bytes().as_ptr() as usize % ALIGNMENT, 0);
            assert_eq!(bounce.bytes().len(), len);
        }
    }

    #[test]
    fn a_file_is_read_and_written_directly_and_holes_read_zero() {
        let dir = std::env::temp_dir().join(format!("qsvs-backing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lun.img");
        let file = File::create(&path).unwrap();
        file.set_len(1 << 20).unwrap();
        drop(file);
        // A tmpfs has no direct I/O: the test needs a real file system.
        let Ok(store) = HostStore::open(&path) else {
            std::fs::remove_dir_all(&dir).unwrap();
            return;
        };
        assert_eq!((store.bytes(), store.block_bytes()), (1 << 20, 512));
        let mut bounce = Bounce::new(8192);
        bounce.bytes().fill(0x5A);
        store.write_at(bounce.bytes(), 4096).unwrap();
        store.flush().unwrap();
        assert_eq!(std::fs::read(&path).unwrap()[4096..4100], [0x5A; 4]);
        if store.discard(4096, 4096).is_ok() {
            store.read_at(bounce.bytes(), 4096).unwrap();
            assert_eq!(bounce.bytes()[..4096], [0u8; 4096]);
            assert_eq!(bounce.bytes()[4096..], [0x5A; 4096]);
        }
        assert!(store.read_at(bounce.bytes(), 1 << 20).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(HostStore::open(Path::new("/dev/null")).is_err());
    }
}
