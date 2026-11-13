// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's configuration for one activation (`docs/VMM.md`):
//! everything `qsvmm` is to know, resolved. The partition manager
//! writes it to `qsvmm`'s standard input when it starts it; `qsvmm`
//! reads no profile and decides nothing.
//!
//! ```toml
//! id = 3
//! name = "PROD"
//! serial = "K2M4103"
//! vcpus = 4
//! threads_per_core = 2
//! pin = [6, 14, 7, 15]     # dedicated processors; left out when shared
//! memory_mib = 2048
//! huge_pages = true
//! side = "recorded"        # a, b or recorded
//! keylock = "normal"       # normal, manual, auto, secure; optional
//! ipl_type = "unattended"  # unattended or attended; optional
//! console = "serial"       # serial or local; optional
//!
//! [[adapter]]
//! slot = 2
//!
//! [[adapter.unit]]
//! lun = 0
//! backing = "/dev/sdb"
//! serial = "PROD-LS"
//! ```
//!
//! It comes from the partition manager, which has validated the
//! profile it was made from; it is checked again here all the same,
//! for what `qsvmm` itself relies on.

use std::path::PathBuf;

use arch::cpu::MAX_VCPUS;
use arch::layout::Ram;
use ipl::Side;
use ipl::lic::{ConsoleMode, IplKind, Keylock, Settings};
use serde::{Deserialize, Serialize};

/// A name's most characters.
pub const NAME_MAX: usize = 10;
/// The most virtual SCSI adapters.
pub const MAX_ADAPTERS: usize = 8;
/// The most units of one adapter.
pub const MAX_UNITS: usize = 64;
/// The highest partition ID.
pub const MAX_ID: u32 = 999;
/// A partition's user and group are this plus its ID.
pub const USER_BASE: u32 = 23_000;
/// The most bytes of a configuration.
pub const MAX_BYTES: u64 = 256 * 1024;
/// A megabyte.
const MIB: u64 = 1 << 20;
/// A huge page.
const HUGE_PAGE: u64 = 2 * MIB;

fn one() -> u32 {
    1
}

/// One unit of an adapter, as written.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Unit {
    /// The logical unit number.
    pub lun: u32,
    /// The host file or block device it is kept in.
    pub backing: PathBuf,
    /// The serial number the guest sees.
    pub serial: String,
}

/// One virtual SCSI adapter, as written.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Adapter {
    /// Its virtual slot.
    pub slot: u32,
    /// Its units.
    #[serde(default, rename = "unit")]
    pub units: Vec<Unit>,
}

/// The configuration, as written.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The partition's ID.
    pub id: u32,
    /// Its name.
    pub name: String,
    /// Its system serial number, for the SMBIOS tables.
    pub serial: String,
    /// Virtual CPUs.
    pub vcpus: u32,
    /// How many of them are the threads of one core.
    #[serde(default = "one")]
    pub threads_per_core: u32,
    /// The host thread each virtual CPU is pinned to; empty for none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pin: Vec<u32>,
    /// Memory, in MiB.
    pub memory_mib: u64,
    /// Whether the memory is 2 MB huge pages the host has reserved.
    #[serde(default)]
    pub huge_pages: bool,
    /// The side: `a`, `b` or `recorded`.
    pub side: String,
    /// The keylock position to write before the IPL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keylock: Option<String>,
    /// The IPL type to write before the IPL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipl_type: Option<String>,
    /// The console mode to write before the IPL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console: Option<String>,
    /// The virtual SCSI adapters.
    #[serde(default, rename = "adapter")]
    pub adapters: Vec<Adapter>,
}

/// A partition, as it is to be activated: a [`Config`] that was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    /// What was written.
    pub config: Config,
    /// Memory, in bytes.
    pub memory_bytes: u64,
    /// The side to IPL from.
    pub side: Side,
    /// What to set in the LIC record before the IPL.
    pub settings: Settings,
    /// The adapters in slot order, each with its units in unit order.
    /// The first unit of the first is the load source.
    pub adapters: Vec<Adapter>,
}

impl Partition {
    /// The user and group the process becomes.
    pub fn user(&self) -> u32 {
        USER_BASE + self.config.id
    }

    /// How many disk units there are.
    pub fn units(&self) -> usize {
        self.adapters.iter().map(|a| a.units.len()).sum()
    }

    /// The load source's backing store.
    pub fn load_source(&self) -> &PathBuf {
        &self.adapters[0].units[0].backing
    }
}

/// Why a configuration was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigError(pub String);

fn refuse<T>(why: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError(why.into()))
}

fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && name.as_bytes()[0].is_ascii_uppercase()
}

fn valid_serial(serial: &str, most: usize) -> bool {
    (1..=most).contains(&serial.len())
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn one_of<T: Copy>(what: &str, value: &str, words: &[(&str, T)]) -> Result<T, ConfigError> {
    words
        .iter()
        .find(|(word, _)| *word == value)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            let all: Vec<&str> = words.iter().map(|(word, _)| *word).collect();
            ConfigError(format!("{what}: {}, not {value:?}", all.join(", ")))
        })
}

/// The adapters that have units, in slot order, each with its units in
/// unit order.
fn adapters(written: &[Adapter]) -> Result<Vec<Adapter>, ConfigError> {
    if written.len() > MAX_ADAPTERS {
        return refuse(format!("adapter: {MAX_ADAPTERS} at most"));
    }
    let mut adapters = written.to_vec();
    adapters.retain(|a| !a.units.is_empty());
    adapters.sort_by_key(|a| a.slot);
    if adapters.is_empty() {
        return refuse("adapter: no unit to IPL from");
    }
    let mut seen: Vec<&Unit> = Vec::new();
    for (i, adapter) in adapters.iter().enumerate() {
        let slot = adapter.slot;
        if i > 0 && adapters[i - 1].slot == slot {
            return refuse(format!("adapter: slot {slot} twice"));
        }
        if adapter.units.len() > MAX_UNITS {
            return refuse(format!("slot {slot}: {MAX_UNITS} units at most"));
        }
        for unit in &adapter.units {
            let what = format!("slot {slot} unit {}", unit.lun);
            if unit.lun as usize >= adapter.units.len() {
                return refuse(format!("{what}: units are numbered from 0 without a gap"));
            }
            if adapter.units.iter().filter(|u| u.lun == unit.lun).count() > 1 {
                return refuse(format!("{what}: twice"));
            }
            if !unit.backing.is_absolute() {
                return refuse(format!("{what}: backing is not an absolute path"));
            }
            if !valid_serial(&unit.serial, 20) {
                return refuse(format!(
                    "{what}: serial is 1 to 20 letters, digits, - and _"
                ));
            }
            if seen.iter().any(|u| u.serial == unit.serial) {
                return refuse(format!("{what}: serial {} is another unit's", unit.serial));
            }
            if seen.iter().any(|u| u.backing == unit.backing) {
                return refuse(format!("{what}: backing is another unit's"));
            }
            seen.push(unit);
        }
    }
    for adapter in &mut adapters {
        adapter.units.sort_by_key(|u| u.lun);
    }
    Ok(adapters)
}

/// Check a configuration.
pub fn check(config: Config) -> Result<Partition, ConfigError> {
    if config.id == 0 || config.id > MAX_ID {
        return refuse(format!("id: 1 to {MAX_ID}"));
    }
    if !valid_name(&config.name) {
        return refuse("name: 1 to 10 of A to Z, 0 to 9 and _, a letter first");
    }
    if !valid_serial(&config.serial, 16) {
        return refuse("serial: 1 to 16 letters and digits");
    }
    let threads = config.threads_per_core;
    if config.vcpus == 0 || config.vcpus > MAX_VCPUS {
        return refuse(format!("vcpus: 1 to {MAX_VCPUS}"));
    }
    if !(1..=2).contains(&threads) || !config.vcpus.is_multiple_of(threads) {
        return refuse("threads_per_core: 1 or 2, and vcpus a multiple of it");
    }
    if !config.pin.is_empty() && config.pin.len() != config.vcpus as usize {
        return refuse("pin: a host thread for each virtual CPU, or none");
    }
    if config.pin.iter().any(|cpu| *cpu >= 4096) {
        return refuse("pin: a host thread's number is below 4096");
    }
    let memory_bytes = config
        .memory_mib
        .checked_mul(MIB)
        .ok_or_else(|| ConfigError("memory_mib: too large".into()))?;
    Ram::for_bytes(memory_bytes).map_err(|e| ConfigError(format!("memory_mib: {e}")))?;
    if config.huge_pages && !memory_bytes.is_multiple_of(HUGE_PAGE) {
        return refuse("memory_mib: huge pages are 2 MiB each");
    }
    let side = one_of(
        "side",
        &config.side,
        &[("a", Side::A), ("b", Side::B), ("recorded", Side::Recorded)],
    )?;
    let keylock = config
        .keylock
        .as_deref()
        .map(|k| {
            one_of(
                "keylock",
                k,
                &[
                    ("normal", Keylock::Normal),
                    ("manual", Keylock::Manual),
                    ("auto", Keylock::Auto),
                    ("secure", Keylock::Secure),
                ],
            )
        })
        .transpose()?;
    let ipl_type = config
        .ipl_type
        .as_deref()
        .map(|t| {
            one_of(
                "ipl_type",
                t,
                &[
                    ("unattended", IplKind::Unattended),
                    ("attended", IplKind::Attended),
                ],
            )
        })
        .transpose()?;
    let console = config
        .console
        .as_deref()
        .map(|c| {
            one_of(
                "console",
                c,
                &[
                    ("serial", ConsoleMode::Serial),
                    ("local", ConsoleMode::Local),
                ],
            )
        })
        .transpose()?;

    let adapters = adapters(&config.adapters)?;
    Ok(Partition {
        memory_bytes,
        side,
        settings: Settings {
            keylock,
            ipl_type,
            console,
        },
        adapters,
        config,
    })
}

/// Read a configuration from its text.
pub fn parse(text: &str) -> Result<Partition, ConfigError> {
    let config: Config = toml::from_str(text).map_err(|e| ConfigError(e.message().to_string()))?;
    check(config)
}

/// A configuration's text.
pub fn render(config: &Config) -> String {
    toml::to_string(config).unwrap_or_default()
}

/// A configuration that is valid, for tests: one adapter in slot 2
/// with the load source `backing`.
pub fn sample(backing: &str) -> Config {
    Config {
        id: 1,
        name: "TESTLPAR".into(),
        serial: "K2M4101".into(),
        vcpus: 2,
        threads_per_core: 1,
        pin: Vec::new(),
        memory_mib: 2048,
        huge_pages: false,
        side: "recorded".into(),
        keylock: None,
        ipl_type: None,
        console: None,
        adapters: vec![Adapter {
            slot: 2,
            units: vec![Unit {
                lun: 0,
                backing: PathBuf::from(backing),
                serial: "TESTLPAR-0".into(),
            }],
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_round_trips_and_is_read_with_everything_set() {
        let mut config = sample("/dev/sdb");
        config.vcpus = 4;
        config.threads_per_core = 2;
        config.pin = vec![6, 14, 7, 15];
        config.huge_pages = true;
        config.side = "b".into();
        config.keylock = Some("manual".into());
        config.ipl_type = Some("attended".into());
        config.console = Some("serial".into());
        config.adapters.push(Adapter {
            slot: 1,
            units: vec![
                Unit {
                    lun: 0,
                    backing: "/dev/sdd".into(),
                    serial: "D3".into(),
                },
                Unit {
                    lun: 1,
                    backing: "/dev/sdc".into(),
                    serial: "D1".into(),
                },
            ],
        });
        let text = render(&config);
        let p = parse(&text).unwrap();
        assert_eq!(p.config, config);
        assert_eq!(p.memory_bytes, 2048 << 20);
        assert_eq!(p.side, Side::B);
        assert_eq!(p.settings.keylock, Some(Keylock::Manual));
        assert_eq!(p.settings.ipl_type, Some(IplKind::Attended));
        assert_eq!(p.settings.console, Some(ConsoleMode::Serial));
        // Slot order, then unit order: the load source is the first.
        assert_eq!(p.adapters[0].slot, 1);
        assert_eq!(p.adapters[0].units[0].lun, 0);
        assert_eq!(p.load_source(), &PathBuf::from("/dev/sdd"));
        assert_eq!(p.units(), 3);
        assert_eq!(p.user(), 23_001);
    }

    #[test]
    fn the_least_configuration_has_defaults() {
        let p = parse(
            "id = 7\nname = \"A\"\nserial = \"X\"\nvcpus = 1\nmemory_mib = 128\n\
             side = \"recorded\"\n[[adapter]]\nslot = 2\n[[adapter.unit]]\nlun = 0\n\
             backing = \"/data/a.img\"\nserial = \"A-0\"\n",
        )
        .unwrap();
        assert_eq!(p.config.threads_per_core, 1);
        assert!(!p.config.huge_pages && p.config.pin.is_empty());
        assert_eq!(p.settings, Settings::default());
        assert_eq!(p.side, Side::Recorded);
    }

    #[test]
    fn what_is_wrong_is_refused_by_name() {
        let refused = |change: &dyn Fn(&mut Config)| {
            let mut config = sample("/dev/sdb");
            change(&mut config);
            check(config).unwrap_err().0
        };
        assert!(refused(&|c| c.id = 0).starts_with("id"));
        assert!(refused(&|c| c.id = 1000).starts_with("id"));
        assert!(refused(&|c| c.name = "lower".into()).starts_with("name"));
        assert!(refused(&|c| c.serial = "no spaces".into()).starts_with("serial"));
        assert!(refused(&|c| c.vcpus = 0).starts_with("vcpus"));
        assert!(refused(&|c| c.vcpus = 241).starts_with("vcpus"));
        assert!(refused(&|c| c.threads_per_core = 3).starts_with("threads_per_core"));
        assert!(
            refused(&|c| {
                c.vcpus = 3;
                c.threads_per_core = 2;
            })
            .starts_with("threads_per_core")
        );
        assert!(refused(&|c| c.pin = vec![1]).starts_with("pin"));
        assert!(refused(&|c| c.pin = vec![1, 9999]).starts_with("pin"));
        assert!(refused(&|c| c.memory_mib = 1).starts_with("memory_mib"));
        assert!(refused(&|c| c.memory_mib = u64::MAX).starts_with("memory_mib"));
        assert!(
            refused(&|c| {
                c.memory_mib = 129;
                c.huge_pages = true;
            })
            .starts_with("memory_mib")
        );
        assert!(refused(&|c| c.side = "d".into()).starts_with("side"));
        assert!(refused(&|c| c.keylock = Some("open".into())).starts_with("keylock"));
        assert!(refused(&|c| c.ipl_type = Some("b".into())).starts_with("ipl_type"));
        assert!(refused(&|c| c.console = Some("vga".into())).starts_with("console"));
        assert!(refused(&|c| c.adapters.clear()).starts_with("adapter"));
        assert!(refused(&|c| c.adapters[0].units.clear()).starts_with("adapter"));
        assert!(refused(&|c| c.adapters.push(c.adapters[0].clone())).contains("twice"));
        assert!(refused(&|c| c.adapters[0].units[0].lun = 1).contains("without a gap"));
        assert!(refused(&|c| c.adapters[0].units[0].backing = "rel".into()).contains("absolute"));
        assert!(refused(&|c| c.adapters[0].units[0].serial = String::new()).contains("serial"));
        assert!(
            refused(&|c| {
                let mut unit = c.adapters[0].units[0].clone();
                unit.lun = 1;
                c.adapters[0].units.push(unit);
            })
            .contains("another unit's")
        );
        // A key the configuration does not know is refused.
        let text = render(&sample("/dev/sdb")).replace("vcpus", "processors");
        assert!(parse(&text).is_err());
    }
}
