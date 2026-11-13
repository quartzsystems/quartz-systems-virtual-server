// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The map of a resynchronization, kept beside the set's label on the
//! load-source disks so that a power loss does not start it over.

use std::io;

use appliance::LoadSourceNames;
use loadsource::disks::{self, LoadSource};
use loadsource::set::resume_spec;
use protect::mirror::ResumeState;
use protect::set::ResumeJournal;

/// The journal on the record partitions of the disks given.
pub struct Journal {
    names: &'static LoadSourceNames,
    sources: Vec<LoadSource>,
}

impl Journal {
    /// The journal on `sources`.
    pub fn on(names: &'static LoadSourceNames, sources: &[LoadSource]) -> Self {
        Self {
            names,
            sources: sources.to_vec(),
        }
    }

    fn write(&self, payload: &[u8]) -> io::Result<()> {
        let mut maps = disks::resume_maps(self.names, &self.sources).map_err(io::Error::other)?;
        maps.write(payload).map(|_| ()).map_err(io::Error::other)
    }
}

impl ResumeJournal for Journal {
    fn load(&mut self, unit_uuid: [u8; 16]) -> Option<ResumeState> {
        let stored = disks::resume_maps(self.names, &self.sources)
            .ok()?
            .read()
            .ok()?;
        ResumeState::decode(&self.names.resume_map_magic, &stored.payload)
            .filter(|s| s.unit_uuid == unit_uuid)
    }

    fn save(&mut self, state: &ResumeState) -> io::Result<()> {
        let bytes = state.encode(&self.names.resume_map_magic);
        // A set too large for the map's room is not kept: after a power
        // loss its resynchronization starts over.
        if bytes.len() + abrec::HEAD_LEN + 8 > resume_spec(self.names).slot_bytes {
            return Ok(());
        }
        self.write(&bytes)
    }

    fn clear(&mut self) -> io::Result<()> {
        // A payload that is no map.
        self.write(&[0u8; 56])
    }
}
