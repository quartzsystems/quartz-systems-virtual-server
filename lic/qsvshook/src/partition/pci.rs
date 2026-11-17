// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A system hostile to the PCI functions it was given whole, for the
//! suite that holds the VMM to its word that configuration space is
//! the guest's to write and nothing it writes reaches the host
//! (`pio-hostile`). It acts only in a test image running as a
//! partition, when one of the partition's disks begins with
//! [`super::PCI_MARKER`]. For each function it lets go of it in its own
//! kernel, then writes every register by name with what no driver
//! writes (command bits, BARs at RAM and at the
//! interrupt window, the ROM, MSI and MSI-X on with garbage, D3, a
//! function-level reset, the link) and every byte, word and double word
//! of the 4 KiB at random. What it did it says on the serial port. The
//! MSI-X table is in a BAR, which this binary cannot map without unsafe
//! code: the VMM's handling of its bytes is the fuzz target's.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// The partition's serial port.
const SERIAL: &str = "/dev/ttyS0";
/// Bytes of PCI Express configuration space.
const SPACE: u64 = 4096;

fn say(line: &str) {
    if let Ok(mut port) = OpenOptions::new().write(true).open(SERIAL) {
        let _ = port.write_all(format!("hostile-pci: {line}\r\n").as_bytes());
    }
}

/// A xorshift: the same garbage every run.
struct Garbage(u32);

impl Garbage {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next().to_le_bytes()[0]).collect()
    }
}

/// The functions given whole: neither bridges nor virtio.
fn given() -> Vec<PathBuf> {
    let root = Path::new("/sys/bus/pci/devices");
    let mut out: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let read = |f: &str| fs::read_to_string(p.join(f)).unwrap_or_default();
            !read("class").trim().starts_with("0x06") && read("vendor").trim() != "0x1af4"
        })
        .collect();
    out.sort();
    out
}

/// One write of `bytes` at `at`: whether it was taken.
fn put(config: &File, at: u64, bytes: &[u8]) -> bool {
    config.write_at(bytes, at).is_ok()
}

fn byte(config: &File, at: u64) -> u8 {
    let mut b = [0u8; 1];
    let _ = config.read_at(&mut b, at);
    b[0]
}

/// The capabilities' places, by ID: a bounded walk of the list.
fn capabilities(config: &File) -> Vec<(u8, u64)> {
    let mut out = Vec::new();
    let mut at = u64::from(byte(config, 0x34) & 0xFC);
    for _ in 0..48 {
        if at < 0x40 {
            break;
        }
        out.push((byte(config, at), at));
        at = u64::from(byte(config, at + 1) & 0xFC);
    }
    out
}

/// Every register by name, with what no driver writes: the writes made.
fn named(config: &File, g: &mut Garbage) -> usize {
    let mut writes: Vec<(u64, Vec<u8>)> = vec![
        // Command: everything on, then nothing, then memory and master.
        (0x04, vec![0xFF, 0xFF]),
        (0x04, vec![0, 0]),
        (0x04, vec![0x06, 0x00]),
        // Status: write one to clear, all of it.
        (0x06, vec![0xFF, 0xFF]),
        // Cache line, latency, BIST start.
        (0x0C, vec![0xFF, 0xFF, 0xFF, 0xFF]),
    ];
    for bar in 0..6u64 {
        let at = 0x10 + bar * 4;
        // Sizing, over the host's RAM, at the interrupt window, at 0.
        writes.push((at, vec![0xFF; 4]));
        writes.push((at, 0x0010_0000u32.to_le_bytes().to_vec()));
        writes.push((at, 0xFEE0_0000u32.to_le_bytes().to_vec()));
        writes.push((at, vec![0; 4]));
    }
    // The expansion ROM: sized and enabled; the interrupt line and pin.
    writes.push((0x30, vec![0xFF; 4]));
    writes.push((0x30, 0xFEE0_0001u32.to_le_bytes().to_vec()));
    writes.push((0x3C, vec![0xFF, 0xFF]));
    for (id, at) in capabilities(config) {
        match id {
            // Power management: D3hot and back, PME on.
            0x01 => {
                writes.push((at + 4, vec![0x03, 0x81]));
                writes.push((at + 4, vec![0x00, 0x81]));
            }
            // MSI: an address in the host's RAM and in the window, every
            // vector, enabled.
            0x05 => {
                writes.push((at + 4, 0x0010_0000u32.to_le_bytes().to_vec()));
                writes.push((at + 8, g.next().to_le_bytes().to_vec()));
                writes.push((at + 12, g.next().to_le_bytes().to_vec()));
                writes.push((at + 2, vec![0x71, 0x01]));
                writes.push((at + 4, 0xFEE0_F00Cu32.to_le_bytes().to_vec()));
            }
            // PCI Express: a function-level reset with every other
            // control bit, the link's controls, the second set.
            0x10 => {
                writes.push((at + 8, vec![0xFF, 0xFF]));
                writes.push((at + 0x10, vec![0xFF, 0xFF]));
                writes.push((at + 0x28, vec![0xFF, 0xFF]));
                writes.push((at + 0x30, vec![0xFF, 0xFF]));
            }
            // MSI-X: on and masked, on and not, a table that moves.
            0x11 => {
                writes.push((at + 2, vec![0xFF, 0xFF]));
                writes.push((at + 2, vec![0x00, 0x80]));
                writes.push((at + 4, vec![0xFF; 4]));
                writes.push((at + 8, vec![0xFF; 4]));
            }
            // A capability's next pointer, at itself.
            _ => writes.push((at + 1, vec![u8::try_from(at).unwrap_or(0x40)])),
        }
    }
    writes
        .iter()
        .filter(|(at, bytes)| put(config, *at, bytes))
        .count()
}

/// Every byte, word and double word of the space, at random: the
/// writes made.
fn every(config: &File, g: &mut Garbage) -> usize {
    let mut writes = 0;
    for width in [1u64, 2, 4] {
        let mut at = 0;
        while at < SPACE {
            let n = usize::try_from(width).unwrap_or(1);
            if put(config, at, &g.bytes(n)) {
                writes += 1;
            }
            at += width;
        }
    }
    // Writes that cross a register, of odd lengths.
    for _ in 0..2000 {
        let at = u64::from(g.next()) % SPACE;
        let n = 1 + usize::try_from(g.next() % 16).unwrap_or(0);
        if put(config, at, &g.bytes(n)) {
            writes += 1;
        }
    }
    writes
}

fn act() {
    let functions = given();
    if functions.is_empty() {
        say("no function given whole");
        return;
    }
    let mut g = Garbage(0x2545_F491);
    let mut total = 0;
    for dev in functions {
        let name = dev
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // The guest's own driver lets go of it first.
        let _ = fs::write(dev.join("driver/unbind"), &name);
        let Ok(config) = OpenOptions::new()
            .read(true)
            .write(true)
            .open(dev.join("config"))
        else {
            say(&format!("{name}: its configuration space does not open"));
            continue;
        };
        let by_name = named(&config, &mut g);
        let random = every(&config, &mut g);
        say(&format!("{name}: {by_name} by name, {random} at random"));
        total += by_name + random;
    }
    say(&format!("done, {total} writes"));
}

/// Act when the partition is marked, from a thread.
pub fn spawn_if_marked() {
    thread::spawn(|| {
        for _ in 0..30 {
            if super::hostile::marked_with(super::PCI_MARKER) {
                act();
                return;
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}
