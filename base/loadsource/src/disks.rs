// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The load-source disks of a running host: every whole disk whose GPT
//! has the appliance's three partitions. There is one, or two when the load
//! source is mirrored; each has its own ESP and its own record
//! partition, so the firmware and PID 1 can start from either alone.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use abrec::AbError;
use appliance::LoadSourceNames;
use diskimg::efi::EspLocation;
use diskimg::layout::ESP_TYPE_GUID;
use uuid::Uuid;

use crate::gpt::{self, Entry};
use crate::layout::{data_type, record_type};
use crate::multi::{At, Copies};
use crate::record::{self, HostRecord};
use crate::set::{self, SetLabel};

const SYS_BLOCK: &str = "/sys/class/block";

/// One load-source disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadSource {
    /// The disk's kernel name, `sda`.
    pub disk: String,
    /// The ESP's node.
    pub esp: PathBuf,
    /// Where the ESP is, for a firmware boot entry.
    pub esp_at: EspLocation,
    /// The record partition's node.
    pub record: PathBuf,
    /// The data partition's node.
    pub data: PathBuf,
    /// The data partition's own GUID: its identity as a set member.
    pub data_guid: Uuid,
    /// The data partition's length.
    pub data_bytes: u64,
}

fn skipped(name: &str) -> bool {
    ["loop", "ram", "sr", "zram", "ublkb", "fd"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// The whole disks the kernel shows, in name order.
pub fn whole_disks() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(SYS_BLOCK)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.path().join("partition").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !skipped(n))
        .collect();
    names.sort();
    names
}

/// A disk's size in bytes; 0 when it is not there.
pub fn disk_bytes(disk: &str) -> u64 {
    fs::read_to_string(Path::new(SYS_BLOCK).join(disk).join("size"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map_or(0, |sectors| sectors * 512)
}

/// The node of partition `index` of `disk`.
pub fn partition_node(disk: &str, index: u32) -> Option<PathBuf> {
    let wanted = index.to_string();
    fs::read_dir(Path::new(SYS_BLOCK).join(disk))
        .ok()?
        .flatten()
        .find(|e| fs::read_to_string(e.path().join("partition")).is_ok_and(|n| n.trim() == wanted))
        .map(|e| Path::new("/dev").join(e.file_name()))
}

/// The disk a partition node is on.
pub fn disk_of(node: &Path) -> Option<String> {
    let name = node.file_name()?.to_str()?;
    let real = fs::canonicalize(Path::new(SYS_BLOCK).join(name)).ok()?;
    let parent = real.parent()?.file_name()?.to_str()?;
    (parent != "block").then(|| parent.to_string())
}

/// `disk` as a load source, when its table is the appliance's.
pub fn load_source(names: &LoadSourceNames, disk: &str) -> Option<LoadSource> {
    let mut file = File::open(Path::new("/dev").join(disk)).ok()?;
    let table = gpt::entries(&mut file).ok()?;
    let of = |ty: Uuid| -> Option<(Entry, PathBuf)> {
        let entry = *table.iter().find(|e| e.type_guid == ty)?;
        Some((entry, partition_node(disk, entry.index)?))
    };
    let (esp, esp_node) = of(ESP_TYPE_GUID)?;
    let (_, record) = of(record_type(names))?;
    let (data, data_node) = of(data_type(names))?;
    Some(LoadSource {
        disk: disk.to_string(),
        esp: esp_node,
        esp_at: EspLocation {
            index: esp.index,
            first_lba: esp.first_lba,
            sectors: esp.last_lba + 1 - esp.first_lba,
            guid: esp.guid,
        },
        record,
        data: data_node,
        data_guid: data.guid,
        data_bytes: data.bytes(),
    })
}

/// Every load-source disk, in name order.
pub fn load_sources(names: &LoadSourceNames) -> Vec<LoadSource> {
    whole_disks()
        .iter()
        .filter_map(|d| load_source(names, d))
        .collect()
}

/// The load-source disks, waited for: some controllers are probed
/// after PID 1 has started. Returns as soon as there is one and the
/// kernel has shown no new disk for a moment.
pub fn load_sources_within(names: &LoadSourceNames, wait: Duration) -> Vec<LoadSource> {
    let give_up = Instant::now() + wait;
    let mut seen = 0;
    loop {
        let found = load_sources(names);
        let disks = whole_disks().len();
        if (!found.is_empty() && disks == seen) || Instant::now() >= give_up {
            return found;
        }
        seen = disks;
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn open(node: &Path) -> Result<File, AbError> {
    Ok(OpenOptions::new().read(true).write(true).open(node)?)
}

/// The record partitions of `sources`, each from byte `base`.
fn windows(sources: &[LoadSource], base: u64) -> Result<Vec<At<File>>, AbError> {
    let devices: Vec<At<File>> = sources
        .iter()
        .filter_map(|s| open(&s.record).ok())
        .map(|f| At::new(f, base))
        .collect();
    if devices.is_empty() {
        return Err(AbError::NotFound);
    }
    Ok(devices)
}

/// The A/B record on every disk of `sources`.
pub fn records(
    names: &LoadSourceNames,
    sources: &[LoadSource],
) -> Result<Copies<At<File>>, AbError> {
    Ok(Copies::new(record::spec(names), windows(sources, 0)?))
}

/// The set label on every disk of `sources`.
pub fn labels(
    names: &LoadSourceNames,
    sources: &[LoadSource],
) -> Result<Copies<At<File>>, AbError> {
    Ok(Copies::new(
        set::label_spec(names),
        windows(sources, set::LABEL_OFFSET)?,
    ))
}

/// The resume map on every disk of `sources`.
pub fn resume_maps(
    names: &LoadSourceNames,
    sources: &[LoadSource],
) -> Result<Copies<At<File>>, AbError> {
    Ok(Copies::new(
        set::resume_spec(names),
        windows(sources, set::RESUME_OFFSET)?,
    ))
}

/// The newest label on `sources`; `None` when the load source is not
/// mirrored.
pub fn label(names: &LoadSourceNames, sources: &[LoadSource]) -> Option<SetLabel> {
    let stored = labels(names, sources).ok()?.read().ok()?;
    SetLabel::decode(&stored.payload)
}

/// The newest A/B record on `sources`.
pub fn record(
    names: &LoadSourceNames,
    sources: &[LoadSource],
) -> Result<(HostRecord, u64), AbError> {
    record::read_all(&mut records(names, sources)?)
}
