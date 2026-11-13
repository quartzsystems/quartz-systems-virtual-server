// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's profile (`docs/PROFILE.md`). Provisional: Phase 4's
//! partition manager owns profiles; until then one is written by hand.
//!
//! ```toml
//! name = "TESTLPAR"
//! processors = 2
//! memory_mib = 2048
//!
//! [ipl]
//! side = "recorded"      # a, b or recorded
//! keylock = "normal"     # normal, manual, auto, secure; optional
//! console = "serial"     # serial or local; optional
//!
//! [load_source]
//! backing = "/dev/sdb"
//! serial = "TESTLPAR-0"  # optional
//!
//! [[disk]]
//! backing = "/dev/sdc"
//! ```

use std::path::PathBuf;

use arch::cpu::MAX_VCPUS;
use arch::layout::Ram;
use ipl::Side;
use ipl::lic::{ConsoleMode, Keylock, Settings};
use serde::Deserialize;

/// A name's most characters.
pub const NAME_MAX: usize = 10;
/// The most disk units, the load source among them.
pub const MAX_DISKS: usize = 64;
/// A megabyte.
const MIB: u64 = 1 << 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    name: String,
    processors: u32,
    memory_mib: u64,
    #[serde(default)]
    ipl: RawIpl,
    load_source: RawDisk,
    #[serde(default)]
    disk: Vec<RawDisk>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawIpl {
    side: Option<String>,
    keylock: Option<String>,
    console: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDisk {
    backing: PathBuf,
    serial: Option<String>,
}

/// One disk unit of a partition: a LUN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// The host file or block device it is kept in.
    pub backing: PathBuf,
    /// The serial number the guest sees. It stays the same for as
    /// long as the profile does.
    pub serial: String,
}

/// A partition, as it is to be activated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// The partition's name: 1 to 10 of `A` to `Z` and `0` to `9`,
    /// a letter first.
    pub name: String,
    /// Virtual processors.
    pub processors: u32,
    /// Memory, in bytes.
    pub memory_bytes: u64,
    /// The side to IPL from.
    pub side: Side,
    /// The keylock position and console mode to set before the IPL.
    pub settings: Settings,
    /// The disk units: the load source first, LUN 0.
    pub disks: Vec<Disk>,
}

/// Why a profile was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ProfileError(pub String);

fn refuse<T>(why: impl Into<String>) -> Result<T, ProfileError> {
    Err(ProfileError(why.into()))
}

/// Whether `name` is a partition's name.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && name.as_bytes()[0].is_ascii_uppercase()
}

fn valid_serial(serial: &str) -> bool {
    (1..=20).contains(&serial.len())
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Read a profile.
pub fn parse(text: &str) -> Result<Profile, ProfileError> {
    let raw: Raw = toml::from_str(text).map_err(|e| ProfileError(e.message().to_string()))?;
    if !valid_name(&raw.name) {
        return refuse("name: 1 to 10 of A to Z and 0 to 9, a letter first");
    }
    if raw.processors == 0 || raw.processors > MAX_VCPUS {
        return refuse(format!("processors: 1 to {MAX_VCPUS}"));
    }
    let memory_bytes = raw
        .memory_mib
        .checked_mul(MIB)
        .ok_or_else(|| ProfileError("memory_mib: too large".into()))?;
    Ram::for_bytes(memory_bytes).map_err(|e| ProfileError(format!("memory_mib: {e}")))?;
    let side = match raw.ipl.side.as_deref() {
        None | Some("recorded") => Side::Recorded,
        Some("a") => Side::A,
        Some("b") => Side::B,
        Some(other) => return refuse(format!("ipl.side: a, b or recorded, not {other:?}")),
    };
    let keylock = match raw.ipl.keylock.as_deref() {
        None => None,
        Some("normal") => Some(Keylock::Normal),
        Some("manual") => Some(Keylock::Manual),
        Some("auto") => Some(Keylock::Auto),
        Some("secure") => Some(Keylock::Secure),
        Some(other) => {
            return refuse(format!(
                "ipl.keylock: normal, manual, auto or secure, not {other:?}"
            ));
        }
    };
    let console = match raw.ipl.console.as_deref() {
        None => None,
        Some("serial") => Some(ConsoleMode::Serial),
        Some("local") => Some(ConsoleMode::Local),
        Some(other) => return refuse(format!("ipl.console: serial or local, not {other:?}")),
    };
    if raw.disk.len() + 1 > MAX_DISKS {
        return refuse(format!("disk: {MAX_DISKS} disk units at most"));
    }
    let mut disks: Vec<Disk> = Vec::with_capacity(raw.disk.len() + 1);
    for (lun, unit) in std::iter::once(raw.load_source).chain(raw.disk).enumerate() {
        let what = if lun == 0 {
            "load_source".to_string()
        } else {
            format!("disk {lun}")
        };
        if !unit.backing.is_absolute() {
            return refuse(format!("{what}: backing is not an absolute path"));
        }
        let serial = unit.serial.unwrap_or_else(|| format!("{}-{lun}", raw.name));
        if !valid_serial(&serial) {
            return refuse(format!(
                "{what}: serial is 1 to 20 letters, digits, - and _"
            ));
        }
        if disks.iter().any(|d| d.serial == serial) {
            return refuse(format!("{what}: serial {serial} is another unit's"));
        }
        if disks.iter().any(|d| d.backing == unit.backing) {
            return refuse(format!("{what}: backing is another unit's"));
        }
        disks.push(Disk {
            backing: unit.backing,
            serial,
        });
    }
    Ok(Profile {
        name: raw.name,
        processors: raw.processors,
        memory_bytes,
        side,
        settings: Settings { keylock, console },
        disks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
name = "TESTLPAR"
processors = 2
memory_mib = 2048

[ipl]
side = "b"
keylock = "manual"
console = "serial"

[load_source]
backing = "/dev/sdb"

[[disk]]
backing = "/dev/sdc"
serial = "UNIT_2"
"#;

    const SMALL: &str =
        "name = \"A\"\nprocessors = 1\nmemory_mib = 128\n[load_source]\nbacking = \"/x\"\n";

    #[test]
    fn a_profile_reads_as_written() {
        let p = parse(FULL).unwrap();
        assert_eq!(p.name, "TESTLPAR");
        assert_eq!((p.processors, p.memory_bytes), (2, 2048 << 20));
        assert_eq!(p.side, Side::B);
        assert_eq!(p.settings.keylock, Some(Keylock::Manual));
        assert_eq!(p.settings.console, Some(ConsoleMode::Serial));
        assert_eq!(p.disks.len(), 2);
        assert_eq!(p.disks[0].serial, "TESTLPAR-0");
        assert_eq!(p.disks[1].serial, "UNIT_2");
        assert_eq!(p.disks[1].backing, PathBuf::from("/dev/sdc"));
    }

    #[test]
    fn what_is_left_out_takes_the_recorded_side_and_sets_nothing() {
        let p = parse(SMALL).unwrap();
        assert_eq!(p.side, Side::Recorded);
        assert!(!p.settings.any());
        assert_eq!(p.disks.len(), 1);
    }

    #[test]
    fn each_bad_value_is_refused_by_name() {
        let refused = |from: &str, to: &str| -> String {
            assert!(FULL.contains(from), "{from}");
            parse(&FULL.replace(from, to)).unwrap_err().0
        };
        assert!(refused("TESTLPAR", "testlpar").starts_with("name"));
        assert!(refused("TESTLPAR", "9LPAR").starts_with("name"));
        assert!(refused("TESTLPAR", "ELEVENCHARS").starts_with("name"));
        assert!(refused("TESTLPAR", "A/B").starts_with("name"));
        assert!(refused("processors = 2", "processors = 0").starts_with("processors"));
        assert!(refused("processors = 2", "processors = 241").starts_with("processors"));
        assert!(refused("memory_mib = 2048", "memory_mib = 64").starts_with("memory_mib"));
        assert!(refused("side = \"b\"", "side = \"d\"").starts_with("ipl.side"));
        assert!(refused("keylock = \"manual\"", "keylock = \"open\"").starts_with("ipl.keylock"));
        assert!(refused("console = \"serial\"", "console = \"lan\"").starts_with("ipl.console"));
        assert!(refused("/dev/sdb", "sdb").starts_with("load_source"));
        assert!(refused("/dev/sdc", "/dev/sdb").starts_with("disk 1"));
        assert!(refused("UNIT_2", "TESTLPAR-0").starts_with("disk 1"));
        assert!(refused("UNIT_2", "has space").starts_with("disk 1"));
        assert!(refused("processors = 2", "processors = 2\nshares = 5").contains("shares"));
        assert!(parse("").is_err());
    }

    #[test]
    fn the_names_a_partition_may_have() {
        assert!(valid_name("A"));
        assert!(valid_name("QS400PROD1"));
        assert!(!valid_name(""));
        assert!(!valid_name("a"));
        assert!(!valid_name("A B"));
        assert!(!valid_name(".."));
    }
}
