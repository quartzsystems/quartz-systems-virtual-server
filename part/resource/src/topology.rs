// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's processors as partitions are given them: cores, each
//! with the threads the kernel schedules on (one, or two with SMT).
//! A partition's processor is one core.

/// One thread of the host, as its inventory has it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thread {
    /// The kernel's number for it, `cpuN`.
    pub cpu: u32,
    /// Its socket.
    pub socket: u32,
    /// Its core's number within the socket.
    pub core: u32,
}

/// One core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Core {
    /// Its socket.
    pub socket: u32,
    /// Its threads, by the kernel's numbers, in order.
    pub cpus: Vec<u32>,
}

/// The host's cores. A core's number is its place here: cores are in
/// the order of their first thread, so core 0 has `cpu0`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    /// The cores.
    pub cores: Vec<Core>,
}

impl Topology {
    /// From the threads the inventory found.
    pub fn of(threads: &[Thread]) -> Self {
        let mut sorted = threads.to_vec();
        sorted.sort_by_key(|t| t.cpu);
        let mut cores: Vec<(u32, u32, Vec<u32>)> = Vec::new();
        for thread in sorted {
            match cores
                .iter_mut()
                .find(|(socket, core, _)| (*socket, *core) == (thread.socket, thread.core))
            {
                Some((_, _, cpus)) => cpus.push(thread.cpu),
                None => cores.push((thread.socket, thread.core, vec![thread.cpu])),
            }
        }
        Self {
            cores: cores
                .into_iter()
                .map(|(socket, _, cpus)| Core { socket, cpus })
                .collect(),
        }
    }

    /// A host of `cores` cores of `threads` threads each, in one
    /// socket, numbered as Linux numbers them: every core's first
    /// thread, then every core's second.
    pub fn uniform(cores: u32, threads: u32) -> Self {
        Self {
            cores: (0..cores)
                .map(|core| Core {
                    socket: 0,
                    cpus: (0..threads).map(|t| core + t * cores).collect(),
                })
                .collect(),
        }
    }

    /// How many sockets.
    pub fn sockets(&self) -> u32 {
        let mut sockets: Vec<u32> = self.cores.iter().map(|c| c.socket).collect();
        sockets.sort_unstable();
        sockets.dedup();
        count(sockets.len())
    }

    /// How many cores.
    pub fn core_count(&self) -> u32 {
        count(self.cores.len())
    }

    /// The threads of a core: the most any core has.
    pub fn threads_per_core(&self) -> u32 {
        count(self.cores.iter().map(|c| c.cpus.len()).max().unwrap_or(1)).max(1)
    }

    /// The threads of `cores`, in the order of the cores.
    pub fn cpus_of(&self, cores: &[u32]) -> Vec<u32> {
        cores
            .iter()
            .filter_map(|core| self.cores.get(*core as usize))
            .flat_map(|core| core.cpus.iter().copied())
            .collect()
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A set of the kernel's CPU numbers as its lists have it: `0-3,8,10-11`.
pub fn cpu_list(cpus: &[u32]) -> String {
    let mut sorted = cpus.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let first = sorted[i];
        let mut end = first;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        ranges.push(if first == end {
            first.to_string()
        } else {
            format!("{first}-{end}")
        });
        i += 1;
    }
    ranges.join(",")
}

/// A kernel CPU list back as its numbers; `None` when it is not one.
/// At most 4096 numbers are taken from it.
pub fn parse_cpu_list(text: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in text.trim().split(',').filter(|p| !p.is_empty()) {
        let (from, to) = part.split_once('-').unwrap_or((part, part));
        let (from, to) = (from.parse::<u32>().ok()?, to.parse::<u32>().ok()?);
        if from > to || (to - from) as usize + cpus.len() >= 4096 {
            return None;
        }
        cpus.extend(from..=to);
    }
    Some(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_of_one_core_are_found_together_wherever_their_numbers_are() {
        // Two sockets of two cores of two threads, numbered as Linux
        // does: the second threads after all the first.
        let mut threads = Vec::new();
        for cpu in 0..8u32 {
            threads.push(Thread {
                cpu,
                socket: (cpu % 4) / 2,
                core: cpu % 2,
            });
        }
        let t = Topology::of(&threads);
        assert_eq!(
            (t.sockets(), t.core_count(), t.threads_per_core()),
            (2, 4, 2)
        );
        assert_eq!(t.cores[0].cpus, [0, 4]);
        assert_eq!(t.cores[3].cpus, [3, 7]);
        assert_eq!(t.cores[2].socket, 1);
        assert_eq!(t.cpus_of(&[1, 3]), [1, 5, 3, 7]);
        assert_eq!(Topology::uniform(4, 2).cores[1].cpus, [1, 5]);
        assert_eq!(Topology::uniform(4, 1).threads_per_core(), 1);
        assert_eq!(Topology::default().threads_per_core(), 1);
    }

    #[test]
    fn cpu_lists_are_the_kernel_s() {
        assert_eq!(cpu_list(&[3, 0, 1, 2, 8, 11, 10]), "0-3,8,10-11");
        assert_eq!(cpu_list(&[5]), "5");
        assert_eq!(cpu_list(&[]), "");
        assert_eq!(
            parse_cpu_list("0-3,8,10-11\n"),
            Some(vec![0, 1, 2, 3, 8, 10, 11])
        );
        assert_eq!(parse_cpu_list(""), Some(vec![]));
        assert_eq!(parse_cpu_list("3-1"), None);
        assert_eq!(parse_cpu_list("a"), None);
        assert_eq!(parse_cpu_list("0-99999"), None);
    }
}
