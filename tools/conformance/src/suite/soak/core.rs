// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A first reading of a partition's dump: the kernel keeps its log as
//! plain text in memory (the printk ring's text records), so the runs
//! of printable bytes that name a fault, a warning or the soak's own
//! checks are found without symbols. What a record names (a warning's
//! place, an oops's registers and code) is where the deeper reading
//! starts.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

/// What marks a record worth reading.
const NEEDLES: &[&str] = &[
    "WARNING:",
    "cut here",
    "BUG:",
    "Oops",
    "general protection",
    "Kernel panic",
    "double fault",
    "stack-protector",
    "RIP:",
    "Code:",
    "CR2:",
    "Kernel Offset",
    "Bad rss-counter",
    "SOAK BAD",
    "Call Trace",
];

/// The shortest run of printable bytes kept.
const SHORTEST: usize = 12;
/// The most records kept.
const MOST: usize = 400;

/// The printable runs of the file at `path` that hold a needle, each
/// with its offset, in the order they are in the file, at most
/// [`MOST`]; the same text is kept once.
pub fn records(path: &Path) -> io::Result<Vec<(u64, String)>> {
    let mut file = File::open(path)?;
    let mut out: Vec<(u64, String)> = Vec::new();
    let mut run: Vec<u8> = Vec::new();
    let mut start = 0u64;
    let mut at = 0u64;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        for &b in &buffer[..n] {
            if (0x20..0x7f).contains(&b) || b == b'\t' {
                if run.is_empty() {
                    start = at;
                }
                run.push(b);
            } else {
                keep(&mut out, start, &run);
                run.clear();
            }
            at += 1;
        }
        if out.len() >= MOST {
            break;
        }
    }
    keep(&mut out, start, &run);
    Ok(out)
}

fn keep(out: &mut Vec<(u64, String)>, start: u64, run: &[u8]) {
    if run.len() < SHORTEST || out.len() >= MOST {
        return;
    }
    let text = String::from_utf8_lossy(run).into_owned();
    if NEEDLES.iter().any(|n| text.contains(n)) && !out.iter().any(|(_, t)| *t == text) {
        out.push((start, text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_runs_that_name_a_fault_are_found_once_each() {
        let dir = std::env::temp_dir().join(format!("soak-core-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("core");
        let mut bytes = vec![0u8; 100];
        bytes.extend_from_slice(b"WARNING: CPU: 0 PID: 1 at mm/page_alloc.c:42");
        bytes.extend_from_slice(&[0, 1, 2]);
        bytes.extend_from_slice(b"an ordinary line of text");
        bytes.push(0);
        bytes.extend_from_slice(b"WARNING: CPU: 0 PID: 1 at mm/page_alloc.c:42");
        bytes.push(0);
        bytes.extend_from_slice(b"RIP: 0010:x");
        std::fs::write(&path, &bytes).unwrap();
        let found = records(&path).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, 100);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
