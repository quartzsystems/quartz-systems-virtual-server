// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `partitions.toml`: the host's settings for its partitions, and its
//! shared processor pools. PID 1 reads the reserve from it to know how
//! much memory to set aside as huge pages; the partition manager owns
//! the rest.

use model::Units;
use qvsmsg::Issued;
use serde::{Deserialize, Serialize};
use state::store::{Document, Migration};

/// The most partitions a host is set to have, unless it says.
pub const DEFAULT_MAX_PARTITIONS: u32 = 64;
/// The memory block size, in MiB, unless the host says.
pub const DEFAULT_BLOCK_MIB: u64 = 16;
/// The cores kept back for the host, unless it says.
pub const DEFAULT_RESERVED_CORES: u32 = 1;
/// The least memory kept back for the host by default, in MiB.
pub const DEFAULT_RESERVE_FLOOR_MIB: u64 = 2048;
/// A huge page, in MiB.
pub const HUGE_PAGE_MIB: u64 = 2;
/// The most shared processor pools beside the default one.
pub const MAX_POOLS: usize = 32;

fn default_max_partitions() -> u32 {
    DEFAULT_MAX_PARTITIONS
}

fn default_block() -> u64 {
    DEFAULT_BLOCK_MIB
}

fn default_reserved_cores() -> u32 {
    DEFAULT_RESERVED_CORES
}

/// `[service]`: how long the host waits for what it asks of a system,
/// and the dumps it keeps of each partition (`docs/SERVICE.md`,
/// `docs/DUMPS.md`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct Service {
    /// Seconds after a delayed shut down's delay before it is forced.
    pub delayed_grace_secs: u32,
    /// Seconds an operating-system shut down is given before it is
    /// forced.
    pub system_timeout_secs: u32,
    /// Seconds each partition is given to shut down through its system
    /// at the host's own shut down, before it is ended immediately.
    pub host_timeout_secs: u32,
    /// Seconds after the kernel is entered within which a system opens
    /// its service channel; after them it is Running without one.
    pub channel_wait_secs: u32,
    /// The most dumps kept of one partition.
    pub max_dumps: u32,
    /// The most MiB the dumps of one partition take together.
    pub max_dump_mib: u64,
}
ipc::wire_struct!(Service {
    delayed_grace_secs,
    system_timeout_secs,
    host_timeout_secs,
    channel_wait_secs,
    max_dumps,
    max_dump_mib
});

impl Default for Service {
    fn default() -> Self {
        Self {
            delayed_grace_secs: 120,
            system_timeout_secs: 300,
            host_timeout_secs: 300,
            channel_wait_secs: 60,
            max_dumps: 3,
            max_dump_mib: 256 * 1024,
        }
    }
}

impl Service {
    /// Whether every value is the default: the table is then left out.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Each value out of its range.
    fn errors(&self, errors: &mut Vec<Issued>) {
        let ranges: [(&str, u64, u64, u64); 6] = [
            (
                "delayed_grace_secs",
                self.delayed_grace_secs.into(),
                0,
                3600,
            ),
            (
                "system_timeout_secs",
                self.system_timeout_secs.into(),
                10,
                3600,
            ),
            ("host_timeout_secs", self.host_timeout_secs.into(), 10, 3600),
            ("channel_wait_secs", self.channel_wait_secs.into(), 5, 600),
            ("max_dumps", self.max_dumps.into(), 1, 32),
            ("max_dump_mib", self.max_dump_mib, 64, u64::MAX),
        ];
        for (name, value, least, most) in ranges {
            if !(least..=most).contains(&value) {
                let range = if most == u64::MAX {
                    format!("{least} or more")
                } else {
                    format!("{least} to {most}")
                };
                errors.push(qvsmsg::SETTING_INVALID.with(&[
                    &format!("service.{name}"),
                    &value.to_string(),
                    &range,
                ]));
            }
        }
    }
}

/// A shared processor pool that has a name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Pool {
    /// Its name.
    pub name: String,
    /// Its cores, by number.
    pub cores: Vec<u32>,
    /// The most processing units its partitions use together; its
    /// cores' worth when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_units: Option<Units>,
}
ipc::wire_struct!(Pool {
    name,
    cores,
    maximum_units
});

/// `partitions.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The most partitions the host may have defined.
    #[serde(default = "default_max_partitions")]
    pub max_partitions: u32,
    /// The memory block size, in MiB.
    #[serde(default = "default_block")]
    pub memory_block_mib: u64,
    /// The cores kept back for the host.
    #[serde(default = "default_reserved_cores")]
    pub reserved_cores: u32,
    /// The memory kept back for the host, in MiB; the larger of 2 GiB
    /// and 5% of the installed memory when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_memory_mib: Option<u64>,
    /// The shared processor pools beside the default one.
    #[serde(default, rename = "pool", skip_serializing_if = "Vec::is_empty")]
    pub pools: Vec<Pool>,
    /// The first three bytes of the MAC addresses the host makes for
    /// partitions' adapters, `xx:xx:xx`: derived once from the host's
    /// serial number and kept, so they never change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_prefix: Option<String>,
    /// The service channel's timeouts and the dumps kept.
    #[serde(default, skip_serializing_if = "Service::is_default")]
    pub service: Service,
}
ipc::wire_struct!(Settings {
    max_partitions,
    memory_block_mib,
    reserved_cores,
    reserved_memory_mib,
    pools,
    mac_prefix,
    service
});

/// The least memory a host may be told to keep for itself, in MiB:
/// with less it does not stay up.
pub const MIN_RESERVE_MIB: u64 = 512;

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_partitions: DEFAULT_MAX_PARTITIONS,
            memory_block_mib: DEFAULT_BLOCK_MIB,
            reserved_cores: DEFAULT_RESERVED_CORES,
            reserved_memory_mib: None,
            pools: Vec::new(),
            mac_prefix: None,
            service: Service::default(),
        }
    }
}

impl Settings {
    /// The memory kept back for a host with `installed_mib` installed.
    pub fn reserve_mib(&self, installed_mib: u64) -> u64 {
        // A file with less than the least is read as the least: the host
        // must still IPL to be told otherwise.
        self.reserved_memory_mib.map_or_else(
            || DEFAULT_RESERVE_FLOOR_MIB.max(installed_mib / 20),
            |mib| mib.max(MIN_RESERVE_MIB),
        )
    }

    /// The memory partitions can be given on such a host: what is left
    /// of the installed memory, in whole blocks.
    pub fn configurable_mib(&self, installed_mib: u64) -> u64 {
        let block = self.memory_block_mib.max(HUGE_PAGE_MIB);
        installed_mib.saturating_sub(self.reserve_mib(installed_mib)) / block * block
    }

    /// The huge pages that memory is.
    pub fn huge_pages(&self, installed_mib: u64) -> u64 {
        self.configurable_mib(installed_mib) / HUGE_PAGE_MIB
    }

    /// Every error of the settings that needs no look at the host.
    pub fn errors(&self) -> Vec<Issued> {
        let mut errors = Vec::new();
        if self.max_partitions == 0 || self.max_partitions > model::definition::MAX_ID {
            errors.push(qvsmsg::SETTING_INVALID.with(&[
                "max_partitions",
                &self.max_partitions.to_string(),
                &format!("1 to {}", model::definition::MAX_ID),
            ]));
        }
        if let Some(mib) = self
            .reserved_memory_mib
            .filter(|mib| *mib < MIN_RESERVE_MIB)
        {
            errors.push(qvsmsg::SETTING_INVALID.with(&[
                "reserved_memory_mib",
                &mib.to_string(),
                &format!("{MIN_RESERVE_MIB} or more"),
            ]));
        }
        let block = self.memory_block_mib;
        if !(HUGE_PAGE_MIB..=1024).contains(&block) || !block.is_power_of_two() {
            errors.push(qvsmsg::SETTING_INVALID.with(&[
                "memory_block_mib",
                &block.to_string(),
                "a power of two from 2 to 1024",
            ]));
        }
        if self.pools.len() > MAX_POOLS {
            errors.push(qvsmsg::SETTING_INVALID.with(&[
                "pool",
                &self.pools.len().to_string(),
                &format!("{MAX_POOLS} pools at most"),
            ]));
        }
        self.service.errors(&mut errors);
        for (i, pool) in self.pools.iter().enumerate() {
            if !model::valid_name(&pool.name) || pool.name == model::profile::DEFAULT_POOL {
                errors.push(qvsmsg::POOL_NAME_INVALID.with(&[&pool.name]));
            }
            if self.pools[..i].iter().any(|p| p.name == pool.name) {
                errors.push(qvsmsg::POOL_EXISTS.with(&[&pool.name]));
            }
        }
        errors
    }
}

impl Document for Settings {
    const FILE: &'static str = "partitions.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        match self.errors().first() {
            Some(error) => Err(error.to_string()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reserve_is_two_gibibytes_or_a_twentieth() {
        let s = Settings::default();
        assert_eq!(s.reserve_mib(8192), 2048);
        assert_eq!(s.reserve_mib(262_144), 13_107);
        // What is left, in whole blocks, and as huge pages.
        assert_eq!(s.configurable_mib(8192), 6144);
        assert_eq!(s.configurable_mib(8000), 5952);
        assert_eq!(s.huge_pages(8192), 3072);
        assert_eq!(s.configurable_mib(1024), 0);
        let set = Settings {
            reserved_memory_mib: Some(1024),
            ..Settings::default()
        };
        assert_eq!(set.configurable_mib(8192), 7168);
    }

    #[test]
    fn the_settings_are_a_state_file_with_defaults() {
        let s: Settings = state::store::parse("schema_version = 1\n").unwrap();
        assert_eq!(s, Settings::default());
        let with_pool = Settings {
            pools: vec![Pool {
                name: "BATCH".into(),
                cores: vec![2, 3],
                maximum_units: Some(Units(150)),
            }],
            ..Settings::default()
        };
        let text = state::store::render(&with_pool).unwrap();
        assert!(text.contains("[[pool]]") && text.contains("maximum_units = 1.5"));
        assert_eq!(state::store::parse::<Settings>(&text).unwrap(), with_pool);
        for bad in [
            "schema_version = 1\nmax_partitions = 0\n",
            "schema_version = 1\nmemory_block_mib = 24\n",
            "schema_version = 1\n[[pool]]\nname = \"DEFAULT\"\ncores = [1]\n",
            "schema_version = 1\n[[pool]]\nname = \"A\"\ncores = [1]\n[[pool]]\nname = \"A\"\ncores = [2]\n",
        ] {
            assert!(state::store::parse::<Settings>(bad).is_err(), "{bad}");
        }
    }
}
