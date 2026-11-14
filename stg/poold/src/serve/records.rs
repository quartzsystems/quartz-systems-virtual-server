// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The records on the pool disks' record partitions: the member label
//! on every disk of a pool, the write-intent maps of a mirrored pool,
//! and the map of a resume under way.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use loadsource::intent::MapStore;
use loadsource::multi::{At, Copies};
use pool::label::{self, PoolLabel};
use pool::layout::{self, LABEL_OFFSET, RESUME_OFFSET};
use pool::names::PoolDiskNames;
use protect::mirror::ResumeState;
use protect::set::ResumeJournal;

fn open(node: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(node)
}

fn at(records: &[PathBuf], base: u64) -> Vec<At<File>> {
    records
        .iter()
        .filter_map(|r| open(r).ok())
        .map(|f| At::new(f, base))
        .collect()
}

/// Write `label` to the record partitions `records`: every disk of the
/// pool that is there. Whether at least one took it.
pub fn write_label(names: &PoolDiskNames, records: &[PathBuf], label: &PoolLabel) -> bool {
    let devices = at(records, LABEL_OFFSET);
    if devices.is_empty() {
        return false;
    }
    Copies::new(label::spec(names), devices)
        .write(&label.encode())
        .is_ok()
}

/// A write-intent map on `records`, from `base`.
pub fn intent_map(names: &PoolDiskNames, records: &[PathBuf], base: u64) -> MapStore<File> {
    let devs = records.iter().filter_map(|r| open(r).ok()).collect();
    MapStore::new(devs, base, names.intent_magic)
}

/// The resume map of a mirrored pool, on its disks' record partitions.
pub struct Journal {
    pub(crate) names: &'static PoolDiskNames,
    pub(crate) records: Vec<PathBuf>,
}

impl Journal {
    fn copies(&self) -> io::Result<Copies<At<File>>> {
        let devices = at(&self.records, RESUME_OFFSET);
        if devices.is_empty() {
            return Err(io::Error::other("no record partition"));
        }
        Ok(Copies::new(layout::resume_spec(self.names), devices))
    }

    fn write(&self, payload: &[u8]) -> io::Result<()> {
        self.copies()?
            .write(payload)
            .map(|_| ())
            .map_err(io::Error::other)
    }
}

impl ResumeJournal for Journal {
    fn load(&mut self, unit_uuid: [u8; 16]) -> Option<ResumeState> {
        let stored = self.copies().ok()?.read().ok()?;
        ResumeState::decode(&self.names.resume_map_magic, &stored.payload)
            .filter(|s| s.unit_uuid == unit_uuid)
    }

    fn save(&mut self, state: &ResumeState) -> io::Result<()> {
        let bytes = state.encode(&self.names.resume_map_magic);
        // A set too large for the map's room is not kept: after a power
        // loss its resume starts over.
        if bytes.len() + abrec::HEAD_LEN + 8 > layout::RESUME_SLOT_BYTES {
            return Ok(());
        }
        self.write(&bytes)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.write(&[0u8; 56])
    }
}
