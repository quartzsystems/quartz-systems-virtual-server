// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's profile (`docs/PROFILE.md`): what an activation is to
//! give the partition. The fields are the ones QS/MC's Create
//! partition wizard collects.

use serde::{Deserialize, Serialize};
use state::store::{Document, Migration};

use crate::units::Units;

/// The name of the pool every shared partition is in unless its
/// profile names another.
pub const DEFAULT_POOL: &str = "DEFAULT";
/// The uncapped weight of a profile that gives none.
pub const DEFAULT_WEIGHT: u32 = 128;
/// The highest uncapped weight.
pub const MAX_WEIGHT: u32 = 255;

/// How a partition gets its processors.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProcessorMode {
    /// Whole cores no other partition uses.
    Dedicated,
    /// Virtual processors over a shared pool's cores.
    Shared,
}
ipc::wire_enum!(ProcessorMode { 0 => Dedicated, 1 => Shared });

/// Whether a shared partition may use more than its processing units.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sharing {
    /// Never more than its units.
    Capped,
    /// Up to its virtual processors when the pool has spare time.
    Uncapped,
}
ipc::wire_enum!(Sharing { 0 => Capped, 1 => Uncapped });

/// `[processors]`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Processors {
    /// Dedicated or shared.
    pub mode: ProcessorMode,
    /// The fewest processors (dedicated) or virtual processors
    /// (shared) the partition is activated with.
    pub minimum: u32,
    /// The number it is given when they are available.
    pub desired: u32,
    /// The most it may ever be given.
    pub maximum: u32,
    /// Shared: the fewest processing units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_units: Option<Units>,
    /// Shared: the units it is given when they are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_units: Option<Units>,
    /// Shared: the most units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_units: Option<Units>,
    /// Shared: capped or uncapped. Capped when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharing: Option<Sharing>,
    /// Shared and uncapped: the weight, 0 to 255.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<u32>,
    /// Shared: the pool. The default pool when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
}
ipc::wire_struct!(Processors {
    mode,
    minimum,
    desired,
    maximum,
    minimum_units,
    desired_units,
    maximum_units,
    sharing,
    weight,
    pool
});

impl Processors {
    /// The pool a shared partition is in.
    pub fn pool_name(&self) -> &str {
        self.pool.as_deref().unwrap_or(DEFAULT_POOL)
    }

    /// The weight in effect: a capped partition's is 0.
    pub fn weight_in_effect(&self) -> u32 {
        match self.sharing {
            Some(Sharing::Uncapped) => self.weight.unwrap_or(DEFAULT_WEIGHT),
            _ => 0,
        }
    }
}

/// `[memory]`, in MiB.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    /// The least the partition is activated with.
    pub minimum_mib: u64,
    /// What it is given when that is available.
    pub desired_mib: u64,
    /// The most it may ever be given.
    pub maximum_mib: u64,
}
ipc::wire_struct!(Memory {
    minimum_mib,
    desired_mib,
    maximum_mib
});

/// One unit of a virtual SCSI adapter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Lun {
    /// The logical unit number.
    pub lun: u32,
    /// What the unit is kept in: a path on the host, a file or a block
    /// device. Phase 5 puts a virtual disk of a storage pool here.
    pub backing: String,
    /// The serial number the guest sees; made from the partition's
    /// when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
}
ipc::wire_struct!(Lun {
    lun,
    backing,
    serial
});

/// `[[scsi]]`: a virtual SCSI adapter.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScsiAdapter {
    /// The virtual slot.
    pub slot: u32,
    /// Its units.
    #[serde(default, rename = "lun")]
    pub luns: Vec<Lun>,
}
ipc::wire_struct!(ScsiAdapter { slot, luns });

/// Where an IPL comes from.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IplSource {
    /// Side A.
    A,
    /// Side B.
    B,
    /// The side the partition itself asked for.
    Recorded,
    /// Installation media. Phase 5.
    D,
}
ipc::wire_enum!(IplSource { 0 => A, 1 => B, 2 => Recorded, 3 => D });

impl IplSource {
    /// As a profile has it.
    pub fn word(self) -> &'static str {
        match self {
            IplSource::A => "a",
            IplSource::B => "b",
            IplSource::Recorded => "recorded",
            IplSource::D => "d",
        }
    }
}

/// Whether an IPL stops for an operator.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IplType {
    /// It runs through.
    Unattended,
    /// It stops at the IPL menu.
    Attended,
}
ipc::wire_enum!(IplType { 0 => Unattended, 1 => Attended });

/// Where the partition's console is.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleMode {
    /// The serial port: the one that gives a partition a console.
    Serial,
    /// The display, which a partition does not have.
    Local,
}
ipc::wire_enum!(ConsoleMode { 0 => Serial, 1 => Local });

/// `[boot]`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Boot {
    /// The IPL source.
    pub source: IplSource,
    /// The IPL type.
    #[serde(rename = "type")]
    pub ipl_type: IplType,
    /// The console mode.
    pub console: ConsoleMode,
}
ipc::wire_struct!(Boot {
    source,
    ipl_type,
    console
});

impl Default for Boot {
    fn default() -> Self {
        Self {
            source: IplSource::Recorded,
            ipl_type: IplType::Unattended,
            console: ConsoleMode::Serial,
        }
    }
}

/// `[[ethernet]]`: a virtual Ethernet adapter. Phase 6.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EthernetAdapter {
    /// The virtual slot.
    pub slot: u32,
    /// The port's VLAN, 1 to 4094.
    pub vlan: u32,
    /// The MAC address, `xx:xx:xx:xx:xx:xx`; the host's choice when
    /// left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
}
ipc::wire_struct!(EthernetAdapter { slot, vlan, mac });

/// `[[optical]]` and `[[tape]]`: a virtual optical or tape device.
/// Phase 5.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MediaDevice {
    /// The virtual slot.
    pub slot: u32,
    /// The media loaded at activation: a path on the host; none when
    /// left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
}
ipc::wire_struct!(MediaDevice { slot, media });

/// `[[physical_io]]`: a device of the host given to the partition.
/// Phase 10.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PhysicalIo {
    /// The device's location code (`docs/LOCATION.md`).
    pub location: String,
    /// Whether the partition cannot be activated without it.
    #[serde(default)]
    pub required: bool,
}
ipc::wire_struct!(PhysicalIo { location, required });

/// A profile.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Its name, one of the partition's.
    pub name: String,
    /// Processors.
    pub processors: Processors,
    /// Memory.
    pub memory: Memory,
    /// Boot.
    #[serde(default)]
    pub boot: Boot,
    /// Virtual SCSI adapters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scsi: Vec<ScsiAdapter>,
    /// Virtual Ethernet adapters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ethernet: Vec<EthernetAdapter>,
    /// Virtual optical devices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optical: Vec<MediaDevice>,
    /// Virtual tape devices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tape: Vec<MediaDevice>,
    /// Physical I/O.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub physical_io: Vec<PhysicalIo>,
}
ipc::wire_struct!(Profile {
    name,
    processors,
    memory,
    boot,
    scsi,
    ethernet,
    optical,
    tape,
    physical_io
});

impl Profile {
    /// The units in LUN order of the adapters in slot order: the
    /// first is the load source.
    pub fn units(&self) -> Vec<(u32, &Lun)> {
        let mut adapters: Vec<&ScsiAdapter> = self.scsi.iter().collect();
        adapters.sort_by_key(|a| a.slot);
        let mut units = Vec::new();
        for adapter in adapters {
            let mut luns: Vec<&Lun> = adapter.luns.iter().collect();
            luns.sort_by_key(|l| l.lun);
            units.extend(luns.into_iter().map(|l| (adapter.slot, l)));
        }
        units
    }
}

impl Document for Profile {
    const FILE: &'static str = "profile.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    /// The state store asks only that the file is a profile: what a
    /// profile may hold is [`crate::validate`]'s, with every error.
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}
