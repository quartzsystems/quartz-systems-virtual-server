// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvshook soak <disk>`: load for the soak of Section 1
//! (`docs/PHASE11.md`), run by a test image that is a partition's
//! system and was given a disk marked `QSVS-SOAK`. Every byte it writes
//! it reads back and checks, so corruption is seen where it happens
//! and not only when a kernel trips over it:
//!
//! - [`memory`]: pages mapped, filled, checked and unmapped on every
//!   processor, mappings changed under running threads (shootdowns),
//!   and short-lived processes that do the same.
//! - [`disk`]: blocks written to the marked disk, flushed, dropped from
//!   the cache and read back.
//! - [`net`]: raw frames broadcast on every adapter and the frames of
//!   other soak partitions checked as they arrive.
//!
//! Every 30 seconds a line `SOAK ok ...` says what was done; a check
//! that fails says `SOAK BAD ...` with the place and the bytes.

pub mod disk;
pub mod memory;
pub mod net;

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// What the marked disk's first sector starts with.
pub const MARKER: &[u8] = b"QSVS-SOAK";

/// The counts every part keeps.
#[derive(Default)]
pub struct Counts {
    /// Mappings filled and checked.
    pub maps: AtomicU64,
    /// Processes that ran and checked their memory.
    pub procs: AtomicU64,
    /// MiB written to the disk and read back.
    pub disk_mib: AtomicU64,
    /// Frames sent.
    pub tx: AtomicU64,
    /// Frames of another soak partition received whole.
    pub rx: AtomicU64,
    /// Checks that failed.
    pub bad: AtomicU64,
}

/// One line on the serial port, whole: the host relays it. Not
/// `/dev/console`, which is the last console the command line names: the
/// display.
pub fn say(line: &str) {
    for port in ["/dev/ttyS0", "/dev/console"] {
        if let Ok(mut console) = OpenOptions::new().write(true).open(port) {
            let _ = console.write_all(format!("{line}\r\n").as_bytes());
            return;
        }
    }
}

/// A check that failed: said and counted.
pub fn bad(counts: &Counts, what: &str) {
    counts.bad.fetch_add(1, Ordering::Relaxed);
    say(&format!("SOAK BAD {what}"));
}

/// The value of word `index` of a fill seeded with `seed`.
pub fn word(seed: u64, index: u64) -> u64 {
    index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed
}

/// The processors this system has.
fn processors() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// `MemAvailable` in MiB.
fn available_mib() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines()
                .find(|l| l.starts_with("MemAvailable:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|k| k.parse::<u64>().ok())
        })
        .map_or(64, |kib| kib / 1024)
}

/// Run the load on `disk` until the system ends.
pub fn run(disk: &str) {
    let counts = Arc::new(Counts::default());
    let cpus = processors();
    // Half of what is free, shared by the processors' threads; the
    // processes take theirs from the rest.
    let per_thread = (available_mib() / 2 / cpus as u64).clamp(4, 256);
    say(&format!(
        "SOAK start {cpus} processor(s), {per_thread} MiB a thread, disk {disk}"
    ));
    for n in 0..cpus {
        let c = counts.clone();
        std::thread::spawn(move || memory::fill_forever(&c, n as u64, per_thread));
    }
    {
        let c = counts.clone();
        std::thread::spawn(move || memory::processes_forever(&c, per_thread / 2));
    }
    {
        let c = counts.clone();
        std::thread::spawn(move || memory::remap_forever(&c));
    }
    {
        let c = counts.clone();
        let disk = disk.to_string();
        std::thread::spawn(move || disk::forever(&c, &disk));
    }
    net::start(&counts);
    let started = Instant::now();
    loop {
        std::thread::sleep(Duration::from_secs(30));
        say(&format!(
            "SOAK ok {}s maps {} procs {} disk {} MiB tx {} rx {} bad {}",
            started.elapsed().as_secs(),
            counts.maps.load(Ordering::Relaxed),
            counts.procs.load(Ordering::Relaxed),
            counts.disk_mib.load(Ordering::Relaxed),
            counts.tx.load(Ordering::Relaxed),
            counts.rx.load(Ordering::Relaxed),
            counts.bad.load(Ordering::Relaxed),
        ));
    }
}

/// `qsvshook soak-child <MiB>`: one short-lived process's fill and check.
pub fn child(mib: u64) -> i32 {
    let counts = Counts::default();
    memory::fill_once(&counts, u64::from(std::process::id()), mib);
    i32::from(counts.bad.load(Ordering::Relaxed) != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fill_differs_by_place_and_by_seed() {
        assert_ne!(word(1, 0), word(1, 1));
        assert_ne!(word(1, 5), word(2, 5));
        assert_eq!(word(7, 9), word(7, 9));
    }
}
