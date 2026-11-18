// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The marked disk under load: past its first MiB (the marker's), MiB
//! blocks written with a pattern of their place and pass, flushed,
//! dropped from the page cache and read back from the device.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::num::NonZeroU64;
use std::os::unix::fs::FileExt;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rustix::fs::{Advice, fadvise};

use super::{Counts, bad, word};

const BLOCK: usize = 1 << 20;
/// The most of the disk used: the run's I/O, not its size, is the point.
const MOST: u64 = 256 << 20;

fn fill(block: &mut [u8], seed: u64) {
    for (i, chunk) in block.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        *chunk = word(seed, i as u64).to_le_bytes();
    }
}

/// The first word of `block` that is not the fill's.
fn differs(block: &[u8], seed: u64) -> Option<(usize, u64)> {
    block
        .as_chunks::<8>()
        .0
        .iter()
        .enumerate()
        .find_map(|(i, c)| {
            let v = u64::from_le_bytes(*c);
            (v != word(seed, i as u64)).then_some((i, v))
        })
}

fn blocks(file: &mut File) -> u64 {
    let end = file.seek(SeekFrom::End(0)).unwrap_or(0);
    (end.min(MOST) / BLOCK as u64).saturating_sub(1)
}

/// For ever: one pass writes every block and reads each back.
pub fn forever(counts: &Counts, path: &str) {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(f) => f,
        Err(e) => return bad(counts, &format!("disk {path} not opened: {e}")),
    };
    let n = blocks(&mut file);
    if n == 0 {
        return bad(counts, &format!("disk {path} too small"));
    }
    let mut out = vec![0u8; BLOCK];
    let mut back = vec![0u8; BLOCK];
    let mut pass = 0u64;
    loop {
        for b in 0..n {
            let at = (b + 1) * BLOCK as u64;
            let seed = (pass << 32) ^ b;
            fill(&mut out, seed);
            if let Err(e) = file.write_all_at(&out, at).and_then(|()| file.sync_data()) {
                bad(counts, &format!("disk write at {at}: {e}"));
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
            let _ = fadvise(&file, at, NonZeroU64::new(BLOCK as u64), Advice::DontNeed);
            if let Err(e) = file.read_exact_at(&mut back, at) {
                bad(counts, &format!("disk read at {at}: {e}"));
                continue;
            }
            if let Some((i, v)) = differs(&back, seed) {
                bad(
                    counts,
                    &format!(
                        "disk block at {at} word {i}: wanted {:016x} found {v:016x}",
                        word(seed, i as u64)
                    ),
                );
            }
            counts.disk_mib.fetch_add(1, Ordering::Relaxed);
        }
        pass += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_read_back_whole_differs_nowhere() {
        let mut block = vec![0u8; 4096];
        fill(&mut block, 11);
        assert_eq!(differs(&block, 11), None);
        block[17] ^= 0x40;
        assert_eq!(differs(&block, 11).map(|(i, _)| i), Some(2));
        assert!(differs(&block, 12).is_some());
    }
}
