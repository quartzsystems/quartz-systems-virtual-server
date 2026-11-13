// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The accounts: what the host has installed, what partitions can be
//! given (configurable: installed less the host's reserve), what is
//! left (available: configurable less what running partitions hold),
//! for processors and memory and for each shared pool; and what an
//! activation gets of it.
//!
//! Processors are counted in processing units, 1.00 a core. Memory is
//! counted in MiB, always whole blocks, always huge pages.

use model::Units;
use model::profile::{DEFAULT_POOL, ProcessorMode, Profile, Sharing};
use qvsmsg::Issued;

use crate::settings::{Pool, Settings};
use crate::topology::Topology;

/// The processors a running partition holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Processors {
    /// Whole cores of its own.
    Dedicated {
        /// The cores, by number.
        cores: Vec<u32>,
    },
    /// Virtual processors over a pool.
    Shared {
        /// The pool.
        pool: String,
        /// Its entitlement.
        units: Units,
        /// Its virtual processors.
        virtuals: u32,
        /// Whether it is kept to its units.
        capped: bool,
        /// Its uncapped weight; 0 when capped.
        weight: u32,
    },
}
ipc::wire_enum!(Processors {
    0 => Dedicated { cores },
    1 => Shared { pool, units, virtuals, capped, weight },
});

impl Processors {
    /// What it counts for, in processing units.
    pub fn units(&self) -> Units {
        match self {
            Processors::Dedicated { cores } => Units::cores(count(cores.len())),
            Processors::Shared { units, .. } => *units,
        }
    }

    /// The processors the partition sees: its cores, or its virtual
    /// processors.
    pub fn processors(&self) -> u32 {
        match self {
            Processors::Dedicated { cores } => count(cores.len()),
            Processors::Shared { virtuals, .. } => *virtuals,
        }
    }
}

/// A running partition's current resources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Current {
    /// Its processors.
    pub processors: Processors,
    /// Its memory, in MiB.
    pub memory_mib: u64,
}
ipc::wire_struct!(Current {
    processors,
    memory_mib
});

/// What a partition holds while it is activated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    /// The partition's ID.
    pub partition: u32,
    /// What it holds.
    pub current: Current,
}

/// Installed, configurable and available, of one resource.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Amount {
    /// What the machine has.
    pub installed: u64,
    /// What partitions can be given: installed less the host's reserve.
    pub configurable: u64,
    /// What no running partition holds.
    pub available: u64,
}
ipc::wire_struct!(Amount {
    installed,
    configurable,
    available
});

/// A shared processor pool, as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolStatus {
    /// Its name.
    pub name: String,
    /// Its cores, by number.
    pub cores: Vec<u32>,
    /// The most units its partitions use together.
    pub maximum: Units,
    /// The units its running partitions are entitled to.
    pub entitled: Units,
    /// What an activation can still be given of it.
    pub available: Units,
    /// Its running partitions.
    pub partitions: u32,
}
ipc::wire_struct!(PoolStatus {
    name,
    cores,
    maximum,
    entitled,
    available,
    partitions
});

/// The host's resources, as they are now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostResources {
    /// Sockets.
    pub sockets: u32,
    /// Cores.
    pub cores: u32,
    /// Threads of a core.
    pub threads_per_core: u32,
    /// Processors, in hundredths of a core.
    pub processors: Amount,
    /// Memory, in MiB.
    pub memory: Amount,
    /// The memory block size, in MiB.
    pub memory_block_mib: u64,
    /// The cores kept back for the host, by number.
    pub reserved_cores: Vec<u32>,
    /// The pools, the default one first.
    pub pools: Vec<PoolStatus>,
}
ipc::wire_struct!(HostResources {
    sockets,
    cores,
    threads_per_core,
    processors,
    memory,
    memory_block_mib,
    reserved_cores,
    pools
});

/// The host, as the accounts see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// Its cores.
    pub topology: Topology,
    /// The memory installed, in MiB.
    pub installed_mib: u64,
    /// The memory set aside for partitions as huge pages, in MiB: what
    /// the settings ask for, or less when the kernel gave fewer pages.
    pub configurable_mib: u64,
    /// The settings and the pools.
    pub settings: Settings,
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn sum(units: impl Iterator<Item = Units>) -> Units {
    Units(units.map(|u| u.0).fold(0, u32::saturating_add))
}

impl Host {
    /// The cores kept back for the host: the lowest numbers.
    pub fn reserved_cores(&self) -> Vec<u32> {
        let cores = self.topology.core_count();
        (0..self.settings.reserved_cores.min(cores)).collect()
    }

    /// The cores partitions can be given.
    pub fn configurable_cores(&self) -> Vec<u32> {
        (self.settings.reserved_cores.min(self.topology.core_count())..self.topology.core_count())
            .collect()
    }

    fn named(&self, name: &str) -> Option<&Pool> {
        self.settings.pools.iter().find(|p| p.name == name)
    }

    /// The cores of the pool `name` while `held` is held; `None` when
    /// there is no such pool. The default pool has every configurable
    /// core that is in no other pool and dedicated to no partition.
    pub fn pool_cores(&self, name: &str, held: &[Held]) -> Option<Vec<u32>> {
        if name != DEFAULT_POOL {
            return self.named(name).map(|p| p.cores.clone());
        }
        let taken = |core: &u32| {
            self.settings.pools.iter().any(|p| p.cores.contains(core))
                || held.iter().any(|h| {
                    matches!(&h.current.processors,
                        Processors::Dedicated { cores } if cores.contains(core))
                })
        };
        Some(
            self.configurable_cores()
                .into_iter()
                .filter(|core| !taken(core))
                .collect(),
        )
    }

    /// The pool `name` as it is while `held` is held.
    pub fn pool(&self, name: &str, held: &[Held]) -> Option<PoolStatus> {
        let cores = self.pool_cores(name, held)?;
        let worth = Units::cores(count(cores.len()));
        let maximum = self
            .named(name)
            .and_then(|p| p.maximum_units)
            .map_or(worth, |max| max.min(worth));
        let members = || {
            held.iter().filter_map(|h| match &h.current.processors {
                Processors::Shared { pool, units, .. } if pool == name => Some(*units),
                _ => None,
            })
        };
        let entitled = sum(members());
        Some(PoolStatus {
            name: name.to_string(),
            cores,
            maximum,
            entitled,
            available: Units(maximum.0.saturating_sub(entitled.0)),
            partitions: count(members().count()),
        })
    }

    /// The names of the pools, the default one first.
    pub fn pool_names(&self) -> Vec<&str> {
        std::iter::once(DEFAULT_POOL)
            .chain(self.settings.pools.iter().map(|p| p.name.as_str()))
            .collect()
    }

    /// The cores of the default pool that a dedicated partition can
    /// still be given: those its shared partitions do not need, for
    /// their units or to have a core for each virtual processor.
    fn free_for_dedication(&self, held: &[Held]) -> Vec<u32> {
        let cores = self.pool_cores(DEFAULT_POOL, held).unwrap_or_default();
        let mut entitled = 0u32;
        let mut widest = 0u32;
        for h in held {
            if let Processors::Shared {
                pool,
                units,
                virtuals,
                ..
            } = &h.current.processors
                && pool == DEFAULT_POOL
            {
                entitled = entitled.saturating_add(units.0);
                widest = widest.max(*virtuals);
            }
        }
        let needed = entitled.div_ceil(100).max(widest) as usize;
        // The highest numbers go first: the default pool keeps the
        // cores beside the host's.
        cores[needed.min(cores.len())..].to_vec()
    }

    /// The memory no running partition holds, in MiB.
    pub fn memory_available(&self, held: &[Held]) -> u64 {
        let used: u64 = held.iter().map(|h| h.current.memory_mib).sum();
        self.configurable_mib.saturating_sub(used)
    }

    /// The host's resources while `held` is held.
    pub fn resources(&self, held: &[Held]) -> HostResources {
        let configurable = Units::cores(count(self.configurable_cores().len()));
        let used = sum(held.iter().map(|h| h.current.processors.units()));
        HostResources {
            sockets: self.topology.sockets(),
            cores: self.topology.core_count(),
            threads_per_core: self.topology.threads_per_core(),
            processors: Amount {
                installed: u64::from(Units::cores(self.topology.core_count()).0),
                configurable: u64::from(configurable.0),
                available: u64::from(configurable.0.saturating_sub(used.0)),
            },
            memory: Amount {
                installed: self.installed_mib,
                configurable: self.configurable_mib,
                available: self.memory_available(held),
            },
            memory_block_mib: self.settings.memory_block_mib,
            reserved_cores: self.reserved_cores(),
            pools: self
                .pool_names()
                .into_iter()
                .filter_map(|name| self.pool(name, held))
                .collect(),
        }
    }

    /// Every error of the pools against this host's cores, with `held`
    /// held: a core that is not there or is the host's, a core in two
    /// pools or dedicated to a running partition, a pool with no core,
    /// a maximum beyond its cores.
    pub fn pool_errors(&self, held: &[Held]) -> Vec<Issued> {
        let mut errors = self.settings.errors();
        let configurable = self.configurable_cores();
        for (i, pool) in self.settings.pools.iter().enumerate() {
            if pool.cores.is_empty() {
                errors.push(qvsmsg::POOL_EMPTY.with(&[&pool.name]));
            }
            for (j, core) in pool.cores.iter().enumerate() {
                let text = core.to_string();
                if !configurable.contains(core) {
                    errors.push(qvsmsg::POOL_CORE_INVALID.with(&[&pool.name, &text]));
                } else if pool.cores[..j].contains(core)
                    || self.settings.pools[..i]
                        .iter()
                        .any(|p| p.cores.contains(core))
                {
                    errors.push(qvsmsg::POOL_CORE_TWICE.with(&[&pool.name, &text]));
                } else if held.iter().any(|h| {
                    matches!(&h.current.processors,
                        Processors::Dedicated { cores } if cores.contains(core))
                }) {
                    errors.push(qvsmsg::POOL_CORE_BUSY.with(&[&pool.name, &text]));
                }
            }
            let worth = Units::cores(count(pool.cores.len()));
            if let Some(max) = pool.maximum_units.filter(|m| *m > worth || m.0 == 0) {
                errors.push(qvsmsg::POOL_MAXIMUM_INVALID.with(&[
                    &pool.name,
                    &max.to_string(),
                    &worth.to_string(),
                ]));
            }
        }
        errors
    }

    /// What an activation of `profile` for the partition `name` gets
    /// while `held` is held: the desired amounts when they are
    /// available, else the most there is that is not below the
    /// minimum; else the message that names the resource, what was
    /// asked and what is available.
    pub fn resolve(&self, held: &[Held], name: &str, profile: &Profile) -> Result<Current, Issued> {
        let p = &profile.processors;
        let processors = match p.mode {
            ProcessorMode::Dedicated => {
                let free = self.free_for_dedication(held);
                let take = (p.desired as usize).min(free.len());
                if take < p.minimum as usize {
                    return Err(qvsmsg::PROCESSORS_NOT_AVAILABLE.with(&[
                        name,
                        &p.minimum.to_string(),
                        &free.len().to_string(),
                    ]));
                }
                Processors::Dedicated {
                    cores: free[free.len() - take..].to_vec(),
                }
            }
            ProcessorMode::Shared => self.resolve_shared(held, name, profile)?,
        };
        let m = &profile.memory;
        let block = self.settings.memory_block_mib.max(1);
        let available = self.memory_available(held) / block * block;
        let memory_mib = m.desired_mib.min(available);
        if memory_mib < m.minimum_mib {
            return Err(qvsmsg::MEMORY_NOT_AVAILABLE.with(&[
                name,
                &m.minimum_mib.to_string(),
                &available.to_string(),
            ]));
        }
        Ok(Current {
            processors,
            memory_mib,
        })
    }

    fn resolve_shared(
        &self,
        held: &[Held],
        name: &str,
        profile: &Profile,
    ) -> Result<Processors, Issued> {
        let p = &profile.processors;
        let pool_name = p.pool_name();
        let pool = self
            .pool(pool_name, held)
            .ok_or_else(|| qvsmsg::POOL_MISSING.with(&[pool_name]))?;
        let (min_units, desired_units) = (
            p.minimum_units.unwrap_or_default(),
            p.desired_units.unwrap_or_default(),
        );
        // A virtual processor needs a core of the pool to run on.
        let cores = count(pool.cores.len());
        let mut virtuals = p.desired.min(cores);
        if virtuals < p.minimum {
            return Err(qvsmsg::VIRTUALS_NOT_AVAILABLE.with(&[
                name,
                &p.minimum.to_string(),
                pool_name,
                &cores.to_string(),
            ]));
        }
        // The units: what is there, and no more than a core for each
        // virtual processor.
        let units = desired_units
            .min(pool.available)
            .min(Units::cores(virtuals));
        if units < min_units || units.0 == 0 {
            return Err(qvsmsg::UNITS_NOT_AVAILABLE.with(&[
                name,
                &min_units.to_string(),
                pool_name,
                &pool.available.to_string(),
            ]));
        }
        // No fewer than 0.05 units for each virtual processor.
        virtuals = virtuals.min(units.0 / Units::LEAST_PER_VIRTUAL.0).max(1);
        let weight = p.weight_in_effect();
        Ok(Processors::Shared {
            pool: pool_name.to_string(),
            units,
            virtuals,
            capped: p.sharing != Some(Sharing::Uncapped) || weight == 0,
            weight,
        })
    }

    /// The virtual CPUs of a partition holding `processors`: how many,
    /// how many of them make a core, and for a dedicated partition the
    /// host thread each is pinned to, in order.
    pub fn vcpus(&self, processors: &Processors) -> Vcpus {
        let threads = self.topology.threads_per_core();
        match processors {
            Processors::Dedicated { cores } => {
                let pin = self.topology.cpus_of(cores);
                Vcpus {
                    count: count(pin.len()),
                    threads_per_core: threads,
                    pin,
                }
            }
            Processors::Shared { virtuals, .. } => Vcpus {
                count: virtuals.saturating_mul(threads),
                threads_per_core: threads,
                pin: Vec::new(),
            },
        }
    }
}

/// A partition's virtual CPUs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vcpus {
    /// How many.
    pub count: u32,
    /// How many of them are the threads of one core.
    pub threads_per_core: u32,
    /// Dedicated: the host thread of each, in order. Shared: empty.
    pub pin: Vec<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::sample;

    /// Eight cores of two threads, 16 GiB, the defaults: one core and
    /// 2 GiB kept back.
    fn host() -> Host {
        let settings = Settings::default();
        Host {
            topology: Topology::uniform(8, 2),
            installed_mib: 16_384,
            configurable_mib: settings.configurable_mib(16_384),
            settings,
        }
    }

    fn hold(partition: u32, host: &Host, held: &[Held], profile: &Profile) -> Held {
        Held {
            partition,
            current: host.resolve(held, "P", profile).unwrap(),
        }
    }

    #[test]
    fn installed_configurable_and_available_differ_by_the_reserve_and_by_what_is_held() {
        let host = host();
        let r = host.resources(&[]);
        assert_eq!((r.sockets, r.cores, r.threads_per_core), (1, 8, 2));
        assert_eq!(r.reserved_cores, [0]);
        assert_eq!(
            r.processors,
            Amount {
                installed: 800,
                configurable: 700,
                available: 700
            }
        );
        assert_eq!(
            r.memory,
            Amount {
                installed: 16_384,
                configurable: 14_336,
                available: 14_336
            }
        );
        assert_eq!(r.pools.len(), 1);
        assert_eq!(r.pools[0].cores, [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(r.pools[0].available, Units(700));

        let mut held = vec![hold(1, &host, &[], &sample::dedicated())];
        held.push(hold(2, &host, &held, &sample::shared()));
        let r = host.resources(&held);
        // Two cores dedicated, 1.50 units shared.
        assert_eq!(r.processors.available, 700 - 200 - 150);
        assert_eq!(r.memory.available, 14_336 - 2 * 2048);
        assert_eq!(r.pools[0].cores, [1, 2, 3, 4, 5]);
        assert_eq!(
            (
                r.pools[0].entitled,
                r.pools[0].available,
                r.pools[0].partitions
            ),
            (Units(150), Units(350), 1)
        );
    }

    #[test]
    fn dedicated_processors_are_whole_cores_from_the_top_with_every_thread_pinned() {
        let host = host();
        let got = host.resolve(&[], "P", &sample::dedicated()).unwrap();
        assert_eq!(got.processors, Processors::Dedicated { cores: vec![6, 7] });
        assert_eq!(got.memory_mib, 2048);
        let vcpus = host.vcpus(&got.processors);
        assert_eq!((vcpus.count, vcpus.threads_per_core), (4, 2));
        assert_eq!(vcpus.pin, [6, 14, 7, 15]);
        // The next takes the two below.
        let held = [Held {
            partition: 1,
            current: got,
        }];
        let next = host.resolve(&held, "P", &sample::dedicated()).unwrap();
        assert_eq!(next.processors, Processors::Dedicated { cores: vec![4, 5] });
    }

    #[test]
    fn the_desired_amount_is_given_when_it_is_there_and_else_what_there_is_down_to_the_minimum() {
        let host = host();
        let mut held = Vec::new();
        // Three partitions of two cores each leave one of seven.
        for id in 1..=3 {
            held.push(hold(id, &host, &held, &sample::dedicated()));
        }
        let fourth = host.resolve(&held, "P", &sample::dedicated()).unwrap();
        assert_eq!(fourth.processors, Processors::Dedicated { cores: vec![1] });
        held.push(Held {
            partition: 4,
            current: fourth,
        });
        // None is left: the minimum of one is not available.
        let refused = host
            .resolve(&held, "FIFTH", &sample::dedicated())
            .unwrap_err();
        assert!(refused.is(&qvsmsg::PROCESSORS_NOT_AVAILABLE));
        assert_eq!(
            refused.text,
            "Partition FIFTH needs 1 dedicated processors; 0 are available."
        );

        // Memory: 14336 MB, four partitions of 2048 hold 8192.
        let mut big = sample::shared();
        big.memory.desired_mib = 8192;
        big.memory.minimum_mib = 4096;
        let lacking = host.resolve(&held[..3], "P", &big).unwrap();
        assert_eq!(lacking.memory_mib, 14_336 - 3 * 2048);
        big.memory.desired_mib = 16_384;
        big.memory.minimum_mib = 12_288;
        let refused = host.resolve(&held[..3], "BIG", &big).unwrap_err();
        assert!(refused.is(&qvsmsg::MEMORY_NOT_AVAILABLE));
        assert_eq!(
            refused.text,
            "Partition BIG needs 12288 MB of memory; 8192 MB are available."
        );
    }

    #[test]
    fn shared_processors_take_units_of_their_pool_and_a_core_for_each_virtual_processor() {
        let host = host();
        let got = host.resolve(&[], "P", &sample::shared()).unwrap();
        assert_eq!(
            got.processors,
            Processors::Shared {
                pool: "DEFAULT".into(),
                units: Units(150),
                virtuals: 2,
                capped: false,
                weight: 128,
            }
        );
        let vcpus = host.vcpus(&got.processors);
        assert_eq!((vcpus.count, vcpus.threads_per_core), (4, 2));
        assert!(vcpus.pin.is_empty());

        // A pool of two cores with a maximum of 1.20 units.
        let mut host = host;
        host.settings.pools.push(Pool {
            name: "SMALL".into(),
            cores: vec![6, 7],
            maximum_units: Some(Units(120)),
        });
        assert_eq!(host.pool_errors(&[]), []);
        let mut p = sample::shared();
        p.processors.pool = Some("SMALL".into());
        p.processors.desired = 4;
        p.processors.minimum = 2;
        p.processors.minimum_units = Some(Units(100));
        let got = host.resolve(&[], "P", &p).unwrap();
        // Two cores: two virtual processors; 1.20 of the 1.50 wanted.
        assert_eq!(
            got.processors,
            Processors::Shared {
                pool: "SMALL".into(),
                units: Units(120),
                virtuals: 2,
                capped: false,
                weight: 128,
            }
        );
        let held = [Held {
            partition: 1,
            current: got,
        }];
        let refused = host.resolve(&held, "NEXT", &p).unwrap_err();
        assert!(refused.is(&qvsmsg::UNITS_NOT_AVAILABLE));
        assert!(
            refused.text.contains("1.00") && refused.text.contains("0.00"),
            "{refused}"
        );
        p.processors.minimum = 3;
        p.processors.minimum_units = Some(Units(15));
        let refused = host.resolve(&[], "WIDE", &p).unwrap_err();
        assert!(refused.is(&qvsmsg::VIRTUALS_NOT_AVAILABLE));
        p.processors.pool = Some("NOPOOL".into());
        assert!(
            host.resolve(&[], "P", &p)
                .unwrap_err()
                .is(&qvsmsg::POOL_MISSING)
        );
        // The default pool no longer has the named pool's cores.
        assert_eq!(host.pool("DEFAULT", &[]).unwrap().cores, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn few_units_keep_few_virtual_processors_and_weight_zero_is_capped() {
        let host = host();
        let mut p = sample::shared();
        p.processors.minimum = 1;
        p.processors.desired = 4;
        p.processors.minimum_units = Some(Units(5));
        p.processors.desired_units = Some(Units(400));
        // Other partitions hold all but 0.12 units of the pool.
        let held = [Held {
            partition: 9,
            current: Current {
                processors: Processors::Shared {
                    pool: "DEFAULT".into(),
                    units: Units(688),
                    virtuals: 7,
                    capped: true,
                    weight: 0,
                },
                memory_mib: 1024,
            },
        }];
        let got = host.resolve(&held, "P", &p).unwrap();
        assert!(matches!(
            got.processors,
            Processors::Shared {
                units: Units(12),
                virtuals: 2,
                ..
            }
        ));
        p.processors.weight = Some(0);
        let got = host.resolve(&[], "P", &p).unwrap();
        assert!(matches!(
            got.processors,
            Processors::Shared {
                capped: true,
                weight: 0,
                ..
            }
        ));
        p.processors.sharing = Some(Sharing::Capped);
        p.processors.weight = None;
        let got = host.resolve(&[], "P", &p).unwrap();
        assert!(matches!(
            got.processors,
            Processors::Shared {
                capped: true,
                weight: 0,
                ..
            }
        ));
    }

    #[test]
    fn shared_partitions_keep_the_cores_they_need_from_dedication() {
        let host = host();
        // 3.20 units over 5 virtual processors in the default pool.
        let shared = Held {
            partition: 1,
            current: Current {
                processors: Processors::Shared {
                    pool: "DEFAULT".into(),
                    units: Units(320),
                    virtuals: 5,
                    capped: true,
                    weight: 0,
                },
                memory_mib: 1024,
            },
        };
        let mut p = sample::dedicated();
        p.processors.desired = 4;
        // Seven cores, five needed for the virtual processors: two.
        let got = host
            .resolve(std::slice::from_ref(&shared), "P", &p)
            .unwrap();
        assert_eq!(got.processors, Processors::Dedicated { cores: vec![6, 7] });
    }

    #[test]
    fn a_pool_s_cores_are_checked_against_the_host() {
        let mut host = host();
        host.settings.pools = vec![
            Pool {
                name: "A".into(),
                cores: vec![0, 2, 2, 9],
                maximum_units: Some(Units(500)),
            },
            Pool {
                name: "B".into(),
                cores: vec![2, 7],
                maximum_units: None,
            },
            Pool {
                name: "C".into(),
                cores: vec![],
                maximum_units: None,
            },
        ];
        let held = [Held {
            partition: 1,
            current: Current {
                processors: Processors::Dedicated { cores: vec![7] },
                memory_mib: 1024,
            },
        }];
        let ids: Vec<String> = host.pool_errors(&held).into_iter().map(|e| e.id).collect();
        assert_eq!(
            ids,
            [
                qvsmsg::POOL_CORE_INVALID.id,
                qvsmsg::POOL_CORE_TWICE.id,
                qvsmsg::POOL_CORE_INVALID.id,
                qvsmsg::POOL_MAXIMUM_INVALID.id,
                qvsmsg::POOL_CORE_TWICE.id,
                qvsmsg::POOL_CORE_BUSY.id,
                qvsmsg::POOL_EMPTY.id,
            ]
        );
    }
}
