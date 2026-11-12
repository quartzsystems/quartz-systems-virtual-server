// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The memory: the total from `/proc/meminfo`, and the huge page sizes
//! the kernel offers under `/sys/kernel/mm/hugepages`.

use std::path::Path;

use state::doc::inventory::Memory;

/// The memory. `meminfo` is the text of `/proc/meminfo`.
pub(crate) fn read(sys: &Path, meminfo: &str) -> Memory {
    let pages = sys.join("kernel/mm/hugepages");
    Memory {
        total_bytes: total(meminfo),
        huge_2m: pages.join("hugepages-2048kB").is_dir(),
        huge_1g: pages.join("hugepages-1048576kB").is_dir(),
    }
}

/// `MemTotal` in bytes; 0 when the line is missing.
fn total(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.split_ascii_whitespace().next()?.parse::<u64>().ok())
        .map_or(0, |kb| kb.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Tree;

    #[test]
    fn the_total_is_memtotal_in_bytes() {
        let text = "MemFree:  1 kB\nMemTotal:       16314140 kB\nMemAvailable:   2 kB\n";
        assert_eq!(total(text), 16_314_140 * 1024);
        assert_eq!(total(""), 0);
        assert_eq!(total("MemTotal: lots kB\n"), 0);
        assert_eq!(total("MemTotal: 18446744073709551615 kB\n"), u64::MAX);
    }

    #[test]
    fn a_huge_page_size_is_available_when_the_kernel_lists_it() {
        let tree = Tree::new("memory-huge");
        let sys = tree.sys();
        let none = read(&sys, "MemTotal: 1024 kB\n");
        assert_eq!(none.total_bytes, 1 << 20);
        assert!(!none.huge_2m && !none.huge_1g);
        tree.dir("sys/kernel/mm/hugepages/hugepages-2048kB");
        let small = read(&sys, "");
        assert!(small.huge_2m && !small.huge_1g);
        tree.dir("sys/kernel/mm/hugepages/hugepages-1048576kB");
        let both = read(&sys, "");
        assert!(both.huge_2m && both.huge_1g);
    }
}
