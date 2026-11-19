// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The processors' threads and where each is: its socket and its core,
//! from `/sys/devices/system/cpu`. What partitions are given cores by.

use std::path::Path;

use crate::CpuThread;
use crate::sysfs::{names, number, numbered};

/// The most threads read: more than any host has.
const MAX_THREADS: usize = 4096;

/// Every thread that is online, in the kernel's order.
pub(crate) fn read(sys: &Path) -> Vec<CpuThread> {
    let dir = sys.join("devices/system/cpu");
    let mut threads: Vec<CpuThread> = names(&dir)
        .iter()
        .filter_map(|name| {
            let cpu = numbered(name, "cpu")?;
            let node = dir.join(name);
            // The boot processor has no `online`: it always is.
            if number::<u32>(node.join("online")) == Some(0) {
                return None;
            }
            let topology = node.join("topology");
            Some(CpuThread {
                cpu,
                // A kernel without topology: one socket, a core each.
                socket: number(topology.join("physical_package_id")).unwrap_or(0),
                core: number(topology.join("core_id")).unwrap_or(cpu),
            })
        })
        .take(MAX_THREADS)
        .collect();
    threads.sort_by_key(|t| t.cpu);
    threads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Tree;

    #[test]
    fn each_online_thread_has_its_socket_and_core() {
        let tree = Tree::new("threads");
        for (cpu, core, online) in [
            (0, 0, None),
            (1, 1, Some(1)),
            (2, 0, Some(1)),
            (3, 1, Some(0)),
        ] {
            let node = format!("sys/devices/system/cpu/cpu{cpu}");
            tree.file(&format!("{node}/topology/physical_package_id"), "0\n");
            tree.file(&format!("{node}/topology/core_id"), format!("{core}\n"));
            if let Some(online) = online {
                tree.file(&format!("{node}/online"), format!("{online}\n"));
            }
        }
        // Not a thread.
        tree.dir("sys/devices/system/cpu/cpufreq");
        tree.file("sys/devices/system/cpu/cpu10/topology/core_id", "5\n");
        let threads = read(&tree.sys());
        let found: Vec<(u32, u32, u32)> =
            threads.iter().map(|t| (t.cpu, t.socket, t.core)).collect();
        assert_eq!(found, [(0, 0, 0), (1, 0, 1), (2, 0, 0), (10, 0, 5)]);
        assert!(read(&tree.sys().join("nowhere")).is_empty());
    }
}
