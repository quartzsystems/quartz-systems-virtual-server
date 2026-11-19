// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The memory: the total from `/proc/meminfo`. The huge page sizes are
//! a product's to add.

use state::doc::inventory::Memory;

/// The memory. `meminfo` is the text of `/proc/meminfo`.
pub(crate) fn read(meminfo: &str) -> Memory {
    Memory {
        total_bytes: total(meminfo),
        ..Memory::default()
    }
}

/// `MemTotal` in bytes; 0 when the line is missing.
pub(crate) fn total(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_ascii_whitespace().next()?.parse::<u64>().ok())
        .map_or(0, |kb| kb.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_total_is_memtotal_in_bytes() {
        let text = "MemFree:  1 kB\nMemTotal:       16314140 kB\nMemAvailable:   2 kB\n";
        assert_eq!(total(text), 16_314_140 * 1024);
        assert_eq!(total(""), 0);
        assert_eq!(total("MemTotal: lots kB\n"), 0);
        assert_eq!(total("MemTotal: 18446744073709551615 kB\n"), u64::MAX);
        assert_eq!(read("MemTotal: 1024 kB\n").total_bytes, 1 << 20);
        assert!(!read("").huge_2m);
    }
}
