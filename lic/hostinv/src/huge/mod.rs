// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The huge page sizes the kernel offers under
//! `/sys/kernel/mm/hugepages`: partitions' memory is 2 MB pages.

use std::path::Path;

use state::doc::inventory::Memory;

/// `memory` with the huge page sizes the kernel offers.
pub(crate) fn fill(sys: &Path, memory: &mut Memory) {
    let pages = sys.join("kernel/mm/hugepages");
    memory.huge_2m = pages.join("hugepages-2048kB").is_dir();
    memory.huge_1g = pages.join("hugepages-1048576kB").is_dir();
}

#[cfg(test)]
mod tests {
    use super::*;
    use inventory::fixture::Tree;

    #[test]
    fn a_huge_page_size_is_available_when_the_kernel_lists_it() {
        let tree = Tree::new("memory-huge");
        let sys = tree.sys();
        let mut memory = Memory {
            total_bytes: 1 << 20,
            ..Memory::default()
        };
        fill(&sys, &mut memory);
        assert_eq!(memory.total_bytes, 1 << 20);
        assert!(!memory.huge_2m && !memory.huge_1g);
        tree.dir("sys/kernel/mm/hugepages/hugepages-2048kB");
        fill(&sys, &mut memory);
        assert!(memory.huge_2m && !memory.huge_1g);
        tree.dir("sys/kernel/mm/hugepages/hugepages-1048576kB");
        fill(&sys, &mut memory);
        assert!(memory.huge_2m && memory.huge_1g);
    }
}
