// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition's serial log: `serial.log` in the partition's log
//! directory, and `serial.log.1` to `serial.log.3` behind it. When the
//! file would grow past its limit it becomes `.1`, the others move up
//! and the oldest goes. A guest that never stops talking so fills a
//! bounded space.
//!
//! The directory is held open and every generation is opened and moved
//! relative to it: the confined VMM sees no file system
//! (`crate::confine`).

use std::fs::{self, File};
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::path::Path;

use rustix::fs::{Mode, OFlags, open, openat, renameat};

/// The log's file name.
pub const FILE: &str = "serial.log";
/// Generations kept behind the current file.
pub const GENERATIONS: u32 = 3;
/// The size a log file is rotated at, when the caller has no other.
pub const LIMIT_BYTES: u64 = 4 << 20;

/// The log.
pub struct Log {
    dir: OwnedFd,
    file: Option<File>,
    written: u64,
    limit: u64,
}

impl Log {
    /// Open the log in `dir`, making the directory, and go on at the
    /// end of what is there.
    pub fn open(dir: &Path, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let dir = open(
            dir,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let file = Self::open_at(&dir, 0)?;
        let written = file.metadata()?.len();
        Ok(Self {
            dir,
            file: Some(file),
            written,
            limit: limit.max(1),
        })
    }

    fn open_at(dir: &OwnedFd, n: u32) -> io::Result<File> {
        let fd = openat(
            dir,
            Self::generation(n),
            OFlags::WRONLY | OFlags::CREATE | OFlags::APPEND | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o644),
        )?;
        Ok(File::from(fd))
    }

    /// The directory's descriptor: the only one a generation is opened
    /// in (`crate::confine`).
    pub fn dir_fd(&self) -> i32 {
        std::os::fd::AsRawFd::as_raw_fd(&self.dir)
    }

    /// The open file, for the caller to hand to another owner.
    pub fn file(&self) -> Option<&File> {
        self.file.as_ref()
    }

    fn generation(n: u32) -> String {
        match n {
            0 => FILE.to_string(),
            n => format!("{FILE}.{n}"),
        }
    }

    fn rotate(&mut self) {
        self.file = None;
        for n in (0..GENERATIONS).rev() {
            // The oldest is written over; one that is missing is not an error.
            let _ = renameat(
                &self.dir,
                Self::generation(n),
                &self.dir,
                Self::generation(n + 1),
            );
        }
        self.file = Self::open_at(&self.dir, 0).ok();
        self.written = 0;
    }

    /// Append `bytes`. A log that cannot be written loses them: the
    /// partition goes on.
    pub fn write(&mut self, bytes: &[u8]) {
        if self.written > 0 && self.written + bytes.len() as u64 > self.limit {
            self.rotate();
        }
        if let Some(file) = &mut self.file
            && file.write_all(bytes).is_ok()
        {
            self.written += bytes.len() as u64;
        }
    }

    /// Have the file on disk.
    pub fn sync(&mut self) {
        if let Some(file) = &self.file {
            let _ = file.sync_data();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_rotates_at_its_limit_and_keeps_three_generations() {
        let dir = std::env::temp_dir().join(format!("qsvmm-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut log = Log::open(&dir, 10).unwrap();
        for line in [
            "one\n", "two\n", "three\n", "four\n", "five\n", "six\n", "seven\n", "eight\n",
            "nine\n",
        ] {
            log.write(line.as_bytes());
        }
        let read = |name: &str| fs::read_to_string(dir.join(name)).unwrap_or_default();
        assert_eq!(read("serial.log"), "nine\n");
        assert_eq!(read("serial.log.1"), "eight\n");
        assert_eq!(read("serial.log.2"), "six\nseven\n");
        assert_eq!(read("serial.log.3"), "four\nfive\n");
        assert!(!dir.join("serial.log.4").exists());
        // A write larger than the limit still lands, in a file of its own.
        log.write(&[b'x'; 50]);
        assert_eq!(read("serial.log").len(), 50);
        // Reopened, the log goes on where it was.
        drop(log);
        let mut log = Log::open(&dir, 100).unwrap();
        log.write(b"more\n");
        assert_eq!(read("serial.log").len(), 55);
        fs::remove_dir_all(&dir).unwrap();
    }
}
