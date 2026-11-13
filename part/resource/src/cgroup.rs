// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The cgroup v2 tree, as a plan: the directories to have and the
//! values to write, in order. Nothing here touches a file system; the
//! partition manager carries a plan out (`docs/RESOURCES.md` has the
//! mapping to the kernel's controls).
//!
//! ```text
//! qsvs/                       everything of QS/VS
//!   host/                     PID 1 and the services: the reserved cores
//!   dedicated/
//!     p<ID>/                  a dedicated partition: its cores, no other's
//!   pools/
//!     <POOL>/                 a shared pool: its cores, its maximum
//!       p<ID>/                a shared partition: its quota and weight
//! ```

use model::Units;
use model::profile::DEFAULT_POOL;

use crate::account::{Current, Held, Host, Processors};
use crate::topology::cpu_list;

/// The root of QS/VS's tree, under the cgroup2 mount.
pub const ROOT: &str = "qsvs";
/// The host services' group.
pub const HOST: &str = "qsvs/host";
/// The parent of the dedicated partitions' groups.
pub const DEDICATED: &str = "qsvs/dedicated";
/// The parent of the pools' groups.
pub const POOLS: &str = "qsvs/pools";
/// The controllers the tree uses.
pub const CONTROLLERS: &str = "+cpuset +cpu +memory +pids +hugetlb";
/// The period of every quota, in microseconds.
pub const PERIOD_US: u64 = 100_000;
/// The memory a `qsvmm` may use for itself, beside the guest's: bytes.
pub const VMM_MEMORY_BYTES: u64 = 512 << 20;
/// The threads a `qsvmm` may have beside one for each virtual CPU.
pub const VMM_THREADS: u32 = 64;
/// The kernel's weight for 1.00 unit at the default uncapped weight.
pub const WEIGHT_PER_UNIT: u64 = 100;
/// The kernel's highest weight.
pub const MAX_KERNEL_WEIGHT: u64 = 10_000;

/// One step of a plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Have this directory.
    Dir(String),
    /// Write the value to the file.
    Write(String, String),
}

fn write(group: &str, file: &str, value: impl Into<String>) -> Step {
    Step::Write(format!("{group}/{file}"), value.into())
}

/// A quota of `units` on a host with `threads` threads a core, as
/// `cpu.max` has it: a core's worth of time is its threads' worth.
pub fn quota(units: Units, threads: u32) -> String {
    let quota_us = u64::from(units.0) * u64::from(threads) * PERIOD_US / 100;
    format!("{} {PERIOD_US}", quota_us.max(1000))
}

/// The kernel's `cpu.weight` for a partition entitled to `units` with
/// the uncapped weight `weight`: in proportion to the units, scaled by
/// the weight, so partitions of equal weight share a pool in
/// proportion to their entitlements. A capped partition competes as
/// one of the default weight.
pub fn kernel_weight(units: Units, weight: u32, capped: bool) -> u64 {
    let weight = if capped {
        u64::from(model::profile::DEFAULT_WEIGHT)
    } else {
        u64::from(weight)
    };
    let scaled = u64::from(units.0) * WEIGHT_PER_UNIT * weight
        / (100 * u64::from(model::profile::DEFAULT_WEIGHT));
    scaled.clamp(1, MAX_KERNEL_WEIGHT)
}

/// The group of the pool `name`.
pub fn pool_group(name: &str) -> String {
    format!("{POOLS}/{name}")
}

/// The group of the partition `id` holding `processors`.
pub fn partition_group(id: u32, processors: &Processors) -> String {
    match processors {
        Processors::Dedicated { .. } => format!("{DEDICATED}/p{id}"),
        Processors::Shared { pool, .. } => format!("{}/p{id}", pool_group(pool)),
    }
}

/// The tree without its partitions, while `held` is held: the host's
/// group on the reserved cores, each pool on its cores with its
/// maximum. Carried out when the partition manager starts, and again
/// whenever a pool's cores change: a dedicated partition takes cores
/// out of the default pool and gives them back.
pub fn base(host: &Host, held: &[Held]) -> Vec<Step> {
    let threads = host.topology.threads_per_core();
    let mut plan = vec![
        Step::Write("cgroup.subtree_control".into(), CONTROLLERS.into()),
        Step::Dir(ROOT.into()),
        write(ROOT, "cgroup.subtree_control", CONTROLLERS),
        Step::Dir(HOST.into()),
    ];
    let reserved = host.topology.cpus_of(&host.reserved_cores());
    if !reserved.is_empty() {
        plan.push(write(HOST, "cpuset.cpus", cpu_list(&reserved)));
    }
    plan.push(Step::Dir(DEDICATED.into()));
    plan.push(write(DEDICATED, "cgroup.subtree_control", CONTROLLERS));
    plan.push(Step::Dir(POOLS.into()));
    plan.push(write(POOLS, "cgroup.subtree_control", CONTROLLERS));
    for name in host.pool_names() {
        let Some(pool) = host.pool(name, held) else {
            continue;
        };
        let group = pool_group(name);
        plan.push(Step::Dir(group.clone()));
        plan.push(write(
            &group,
            "cpuset.cpus",
            cpu_list(&host.topology.cpus_of(&pool.cores)),
        ));
        // The default pool's maximum is its cores, which the cpuset
        // already is.
        let max = if name == DEFAULT_POOL {
            format!("max {PERIOD_US}")
        } else {
            quota(pool.maximum, threads)
        };
        plan.push(write(&group, "cpu.max", max));
        plan.push(write(&group, "cgroup.subtree_control", CONTROLLERS));
    }
    plan
}

/// The group of the partition `id` for what it is to hold: its cores
/// or its quota and weight, the huge pages it may map, and the limits
/// that keep a misbehaving `qsvmm` from starving the host. The
/// partition manager puts `qsvmm` in it before it runs.
pub fn partition(host: &Host, id: u32, current: &Current) -> Vec<Step> {
    let threads = host.topology.threads_per_core();
    let vcpus = host.vcpus(&current.processors);
    let group = partition_group(id, &current.processors);
    let mut plan = vec![Step::Dir(group.clone())];
    match &current.processors {
        Processors::Dedicated { .. } => {
            plan.push(write(&group, "cpuset.cpus", cpu_list(&vcpus.pin)));
        }
        Processors::Shared {
            units,
            virtuals,
            capped,
            weight,
            ..
        } => {
            // Capped: its units and no more. Uncapped: as much as its
            // virtual processors can run.
            let most = if *capped {
                *units
            } else {
                Units::cores(*virtuals)
            };
            plan.push(write(&group, "cpu.max", quota(most, threads)));
            plan.push(write(
                &group,
                "cpu.weight",
                kernel_weight(*units, *weight, *capped).to_string(),
            ));
        }
    }
    let memory_bytes = current.memory_mib << 20;
    plan.push(write(&group, "hugetlb.2MB.max", memory_bytes.to_string()));
    plan.push(write(&group, "memory.max", VMM_MEMORY_BYTES.to_string()));
    plan.push(write(
        &group,
        "pids.max",
        (VMM_THREADS + vcpus.count).to_string(),
    ));
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Pool, Settings};
    use crate::topology::Topology;

    fn host() -> Host {
        let mut settings = Settings::default();
        settings.pools.push(Pool {
            name: "BATCH".into(),
            cores: vec![6, 7],
            maximum_units: Some(Units(150)),
        });
        Host {
            topology: Topology::uniform(8, 2),
            installed_mib: 16_384,
            configurable_mib: 14_336,
            settings,
        }
    }

    fn value<'a>(plan: &'a [Step], file: &str) -> Option<&'a str> {
        plan.iter().find_map(|step| match step {
            Step::Write(path, value) if path == file => Some(value.as_str()),
            _ => None,
        })
    }

    #[test]
    fn a_unit_is_a_core_s_worth_of_time_which_is_its_threads_worth() {
        assert_eq!(quota(Units(50), 1), "50000 100000");
        assert_eq!(quota(Units(50), 2), "100000 100000");
        assert_eq!(quota(Units(200), 2), "400000 100000");
        assert_eq!(quota(Units(5), 1), "5000 100000");
    }

    #[test]
    fn the_kernel_s_weight_is_the_entitlement_scaled_by_the_uncapped_weight() {
        // Equal weights: in proportion to the units.
        assert_eq!(kernel_weight(Units(100), 128, false), 100);
        assert_eq!(kernel_weight(Units(50), 128, false), 50);
        assert_eq!(kernel_weight(Units(300), 128, false), 300);
        // A capped partition competes at the default weight.
        assert_eq!(kernel_weight(Units(50), 0, true), 50);
        // Unequal weights scale it.
        assert_eq!(kernel_weight(Units(100), 255, false), 199);
        assert_eq!(kernel_weight(Units(100), 64, false), 50);
        assert_eq!(kernel_weight(Units(5), 1, false), 1);
        assert_eq!(kernel_weight(Units(24_000), 255, false), MAX_KERNEL_WEIGHT);
    }

    #[test]
    fn the_base_tree_has_the_host_on_its_cores_and_each_pool_on_its_own() {
        let host = host();
        let plan = base(&host, &[]);
        assert_eq!(
            plan[0],
            Step::Write("cgroup.subtree_control".into(), CONTROLLERS.into())
        );
        assert_eq!(value(&plan, "qsvs/host/cpuset.cpus"), Some("0,8"));
        assert_eq!(
            value(&plan, "qsvs/pools/DEFAULT/cpuset.cpus"),
            Some("1-5,9-13")
        );
        assert_eq!(
            value(&plan, "qsvs/pools/DEFAULT/cpu.max"),
            Some("max 100000")
        );
        assert_eq!(
            value(&plan, "qsvs/pools/BATCH/cpuset.cpus"),
            Some("6-7,14-15")
        );
        assert_eq!(
            value(&plan, "qsvs/pools/BATCH/cpu.max"),
            Some("300000 100000")
        );
        // A directory is made before anything is written in it.
        for (i, step) in plan.iter().enumerate() {
            if let Step::Write(path, _) = step
                && let Some((dir, _)) = path.rsplit_once('/')
            {
                assert!(plan[..i].contains(&Step::Dir(dir.to_string())), "{path}");
            }
        }
        // A dedicated partition's cores leave the default pool.
        let held = [Held {
            partition: 1,
            current: Current {
                processors: Processors::Dedicated { cores: vec![4, 5] },
                memory_mib: 1024,
            },
        }];
        let plan = base(&host, &held);
        assert_eq!(
            value(&plan, "qsvs/pools/DEFAULT/cpuset.cpus"),
            Some("1-3,9-11")
        );
    }

    #[test]
    fn a_partition_s_group_has_its_cores_or_its_quota_and_the_vmm_s_limits() {
        let host = host();
        let dedicated = Current {
            processors: Processors::Dedicated { cores: vec![4, 5] },
            memory_mib: 2048,
        };
        let plan = partition(&host, 3, &dedicated);
        assert_eq!(plan[0], Step::Dir("qsvs/dedicated/p3".into()));
        assert_eq!(
            value(&plan, "qsvs/dedicated/p3/cpuset.cpus"),
            Some("4-5,12-13")
        );
        assert_eq!(
            value(&plan, "qsvs/dedicated/p3/hugetlb.2MB.max"),
            Some("2147483648")
        );
        assert_eq!(
            value(&plan, "qsvs/dedicated/p3/memory.max"),
            Some("536870912")
        );
        assert_eq!(value(&plan, "qsvs/dedicated/p3/pids.max"), Some("68"));
        assert_eq!(value(&plan, "qsvs/dedicated/p3/cpu.max"), None);

        let shared = |capped, weight| Current {
            processors: Processors::Shared {
                pool: "BATCH".into(),
                units: Units(50),
                virtuals: 1,
                capped,
                weight,
            },
            memory_mib: 1024,
        };
        let capped = partition(&host, 4, &shared(true, 0));
        assert_eq!(capped[0], Step::Dir("qsvs/pools/BATCH/p4".into()));
        assert_eq!(
            value(&capped, "qsvs/pools/BATCH/p4/cpu.max"),
            Some("100000 100000")
        );
        assert_eq!(value(&capped, "qsvs/pools/BATCH/p4/cpu.weight"), Some("50"));
        assert_eq!(value(&capped, "qsvs/pools/BATCH/p4/cpuset.cpus"), None);
        let uncapped = partition(&host, 4, &shared(false, 255));
        assert_eq!(
            value(&uncapped, "qsvs/pools/BATCH/p4/cpu.max"),
            Some("200000 100000")
        );
        assert_eq!(
            value(&uncapped, "qsvs/pools/BATCH/p4/cpu.weight"),
            Some("99")
        );
    }
}
