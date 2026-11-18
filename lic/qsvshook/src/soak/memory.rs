// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Memory under load: large buffers (the allocator maps and unmaps each
//! one whole) filled with a pattern of their place and checked, on
//! every processor at once, so mappings come and go under running
//! threads and every unmap is a shootdown of the others' translations;
//! and short-lived processes that do the same and exit, so whole
//! address spaces are torn down.

use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{Counts, bad, word};

const MIB_WORDS: usize = (1 << 20) / 8;

/// Fill `mib` MiB seeded with `seed`, check every word, and free it.
pub fn fill_once(counts: &Counts, seed: u64, mib: u64) {
    let words = usize::try_from(mib).unwrap_or(1).max(1) * MIB_WORDS;
    let mut buffer: Vec<u64> = Vec::with_capacity(words);
    buffer.extend((0..words as u64).map(|i| word(seed, i)));
    check(counts, &buffer, seed, "memory");
    counts.maps.fetch_add(1, Ordering::Relaxed);
}

/// Every word of `buffer` against the fill: the first that differs,
/// with its neighbours, is said.
fn check(counts: &Counts, buffer: &[u64], seed: u64, what: &str) {
    let Some(at) = (0..buffer.len()).find(|&i| buffer[i] != word(seed, i as u64)) else {
        return;
    };
    let wrong = buffer
        .iter()
        .enumerate()
        .filter(|(i, v)| **v != word(seed, *i as u64))
        .count();
    let from = at.saturating_sub(2);
    let near: Vec<String> = buffer[from..(at + 6).min(buffer.len())]
        .iter()
        .map(|v| format!("{v:016x}"))
        .collect();
    bad(
        counts,
        &format!(
            "{what} word {at} of {} at {:p}: wanted {:016x} found {:016x}; {wrong} word(s) wrong; \
             from word {from}: {}",
            buffer.len(),
            &raw const buffer[at],
            word(seed, at as u64),
            buffer[at],
            near.join(" ")
        ),
    );
}

/// One processor's share, for ever: a buffer filled, kept a moment so
/// the others unmap theirs around it, checked again, and freed.
pub fn fill_forever(counts: &Counts, thread: u64, mib: u64) {
    let mut pass = 0u64;
    loop {
        let seed = (thread << 48) ^ pass;
        let words = usize::try_from(mib).unwrap_or(1).max(1) * MIB_WORDS;
        let mut buffer: Vec<u64> = Vec::with_capacity(words);
        buffer.extend((0..words as u64).map(|i| word(seed, i)));
        check(counts, &buffer, seed, "memory");
        std::thread::sleep(Duration::from_millis(50));
        check(counts, &buffer, seed, "memory, kept");
        drop(buffer);
        counts.maps.fetch_add(1, Ordering::Relaxed);
        pass += 1;
    }
}

/// Small buffers made and freed as fast as can be: a stream of maps and
/// unmaps, each a shootdown of the other processors.
pub fn remap_forever(counts: &Counts) {
    let mut pass = 0u64;
    loop {
        for size in [2usize, 3, 5] {
            let words = size * MIB_WORDS / 4;
            let mut buffer: Vec<u64> = Vec::with_capacity(words);
            buffer.extend((0..words as u64).step_by(512).map(|i| word(pass, i)));
            if buffer
                .iter()
                .enumerate()
                .any(|(n, v)| *v != word(pass, n as u64 * 512))
            {
                bad(counts, "memory, remapped: a word differs");
            }
        }
        pass += 1;
    }
}

/// Processes, one after the other, each filling and checking `mib` MiB
/// and exiting: its exit status says whether its check passed.
pub fn processes_forever(counts: &Counts, mib: u64) {
    let me = std::env::current_exe().unwrap_or_else(|_| "/qsvs/bin/qsvshook".into());
    loop {
        match Command::new(&me)
            .args(["soak-child", &mib.max(1).to_string()])
            .status()
        {
            Ok(s) if s.success() => {
                counts.procs.fetch_add(1, Ordering::Relaxed);
            }
            Ok(s) => bad(counts, &format!("a process's memory: {s}")),
            Err(e) => {
                bad(counts, &format!("a process did not start: {e}"));
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_whole_fill_is_quiet_and_a_changed_word_is_counted() {
        let counts = Counts::default();
        fill_once(&counts, 3, 1);
        assert_eq!(counts.bad.load(Ordering::Relaxed), 0);
        assert_eq!(counts.maps.load(Ordering::Relaxed), 1);
        let mut buffer: Vec<u64> = (0..64).map(|i| word(9, i)).collect();
        buffer[10] ^= 1;
        check(&counts, &buffer, 9, "test");
        assert_eq!(counts.bad.load(Ordering::Relaxed), 1);
    }
}
