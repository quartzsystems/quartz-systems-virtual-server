// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The memory step: the memory partitions can be given is set aside
//! as 2 MB huge pages, early in the IPL, before anything has
//! fragmented memory. How much is the installed memory less the
//! host's reserve (`partitions.toml`, `docs/RESOURCES.md`). The kernel
//! may give fewer pages than asked: the host then has that much less
//! to configure, and says so.

use std::fs;
use std::path::Path;

use resource::Settings;
use resource::settings::HUGE_PAGE_MIB;
use state::Store;

/// The kernel's count of 2 MB huge pages.
pub const NR_HUGEPAGES: &str = "/sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages";

/// What the step did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reserved {
    /// The pages asked for.
    pub wanted: u64,
    /// The pages the kernel gave.
    pub given: u64,
}

impl Reserved {
    /// What a code says of it.
    pub fn detail(self) -> String {
        if self.given == self.wanted {
            format!("{} pages, {} MiB", self.given, self.given * HUGE_PAGE_MIB)
        } else {
            format!(
                "{} of {} pages, {} MiB",
                self.given,
                self.wanted,
                self.given * HUGE_PAGE_MIB
            )
        }
    }
}

/// Ask the kernel, through the file `nr_hugepages`, for the pages the
/// settings in `store` leave for partitions of `installed_mib`.
pub fn reserve(store: &Store, installed_mib: u64, nr_hugepages: &Path) -> Reserved {
    // Settings that are refused are the partition manager's to report:
    // the defaults keep back enough for the host either way.
    let settings = store.load::<Settings>().ok().flatten().unwrap_or_default();
    let wanted = settings.huge_pages(installed_mib);
    let _ = fs::write(nr_hugepages, wanted.to_string());
    let given = fs::read_to_string(nr_hugepages)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0);
    Reserved { wanted, given }
}

/// Memory written to and so held, `mib` of it: what the kernel cannot
/// make huge pages of while this is kept.
#[cfg(test_hooks)]
pub fn hold(mib: u64) -> Vec<u8> {
    let bytes = usize::try_from(mib << 20).unwrap_or(0);
    std::hint::black_box(vec![1u8; bytes])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pages_asked_for_are_what_the_reserve_leaves() {
        let dir = std::env::temp_dir().join(format!("qsvsinit-memory-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = Store::at(dir.join("state"));
        let file = dir.join("nr_hugepages");
        // 8 GiB, the default reserve of 2 GiB: 6144 MiB.
        let got = reserve(&store, 8192, &file);
        assert_eq!(
            got,
            Reserved {
                wanted: 3072,
                given: 3072
            }
        );
        assert_eq!(got.detail(), "3072 pages, 6144 MiB");
        // The host's own setting.
        let settings = Settings {
            reserved_memory_mib: Some(1024),
            ..Settings::default()
        };
        store.save(&settings).unwrap();
        assert_eq!(reserve(&store, 8192, &file).wanted, 3584);
        // A kernel that gives fewer, or has no huge pages at all.
        let short = Reserved {
            wanted: 3584,
            given: 3000,
        };
        assert_eq!(short.detail(), "3000 of 3584 pages, 6000 MiB");
        let none = reserve(&store, 8192, &dir.join("nowhere/nr_hugepages"));
        assert_eq!((none.wanted, none.given), (3584, 0));
        // A host too small to have anything left.
        assert_eq!(reserve(&Store::at(dir.join("none")), 1024, &file).wanted, 0);
        fs::remove_dir_all(&dir).unwrap();
    }
}
