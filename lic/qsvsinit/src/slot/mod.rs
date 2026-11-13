// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The slot rule. The firmware chooses the slot by boot entry; the A/B
//! record holds the default, the slot on trial and what was last
//! confirmed in each. A slot booted on trial becomes the default, in
//! the record and in `BootOrder`, once the base services are all
//! ready. One that never gets there was only `BootNext`: the next IPL
//! is on the old default, clears the pending field and says so.
//! Nothing here writes a UKI; that is the update phase.

#[cfg(target_os = "linux")]
pub use on_host::{Slots, at_ipl};

#[cfg(target_os = "linux")]
mod on_host {
    use std::fs::File;
    use std::path::Path;

    use abrec::Slot;
    use diskimg::efi::EspLocation;
    use loadsource::boot::{self, EFIVARS};
    use loadsource::disks::{self, LoadSource};
    use loadsource::multi::{At, Copies};
    use loadsource::record::{self, HostRecord, Ipl, letter};
    use refcode::RefCode;

    /// The record and what this IPL is by it.
    pub struct Slots {
        /// The slot this IPL came from.
        pub booted: Slot,
        /// The record as it stands, when it could be read or made.
        pub record: Option<HostRecord>,
        /// What this IPL is.
        pub ipl: Ipl,
        /// The record on every load-source disk.
        part: Option<Copies<At<File>>>,
        /// The ESP of every load-source disk.
        esps: Vec<EspLocation>,
    }

    /// The record step of an IPL from `booted` on the load-source
    /// disks `sources`: read the record (the newest on any disk), note
    /// what this IPL is, and write the record back, to every disk, when
    /// that changes it. `post` is given each code with its detail.
    pub fn at_ipl(
        booted: Slot,
        sources: &[LoadSource],
        mut post: impl FnMut(&RefCode, &str),
    ) -> Slots {
        let mut slots = Slots {
            booted,
            record: None,
            ipl: Ipl::Normal,
            part: None,
            esps: sources.iter().map(|s| s.esp_at).collect(),
        };
        let mut part = match disks::records(&release::LOAD_SOURCE, sources) {
            Ok(part) => part,
            Err(e) => {
                post(&refcode::RECORD_INVALID, &format!("not read: {e}"));
                return slots;
            }
        };
        let read = record::read_all(&mut part);
        let (ipl, now) = match &read {
            Ok((found, _)) => found.at_ipl(booted),
            // No copy is a record: start one from the slot that booted.
            Err(_) => (Ipl::Normal, HostRecord::new(booted)),
        };
        // A disk whose copy is behind (it was away, or it is new) is
        // brought to the record's sequence too.
        let behind = part
            .each()
            .iter()
            .any(|c| c.as_ref().map(|c| c.sequence) != read.as_ref().ok().map(|r| r.1));
        let written = match &read {
            Ok((found, _)) if *found == now && !behind => Ok(()),
            _ => record::write_all(&mut part, now).map(|_| ()),
        };
        match (&read, &written) {
            (Err(e), Ok(())) => post(
                &refcode::RECORD_INVALID,
                &format!("{e}; a new one is written, {}", now.summary()),
            ),
            (Err(e), Err(w)) => post(&refcode::RECORD_INVALID, &format!("{e}; not written: {w}")),
            (Ok(_), Err(w)) => post(&refcode::RECORD_INVALID, &format!("not written: {w}")),
            (Ok(_), Ok(())) => {}
        }
        post(&refcode::RECORD_READ, &now.summary());
        if let Ipl::FellBack { pending } = ipl {
            post(
                &refcode::SLOT_FELL_BACK,
                &format!(
                    "slot {}; this IPL is on slot {}",
                    letter(pending),
                    letter(booted)
                ),
            );
        }
        slots.record = Some(now);
        slots.ipl = ipl;
        slots.part = Some(part);
        slots
    }

    impl Slots {
        /// The base is ready. A slot on trial is confirmed: the record
        /// first, then the firmware's `BootOrder`. Any other IPL only
        /// notes its release in the record.
        pub fn base_ready(&mut self, mut post: impl FnMut(&RefCode, &str)) {
            let (Some(record), Some(part)) = (self.record, self.part.as_mut()) else {
                return;
            };
            let release = release::RELEASE;
            let trial = self.ipl == Ipl::Trial;
            let next = if trial {
                record.confirm(self.booted, release)
            } else {
                record.with_release(self.booted, release)
            };
            if next != record {
                if let Err(e) = record::write_all(part, next) {
                    post(&refcode::RECORD_INVALID, &format!("not written: {e}"));
                    return;
                }
                self.record = Some(next);
            }
            if !trial {
                return;
            }
            self.ipl = Ipl::Normal;
            let order = boot::make_default(
                Path::new(EFIVARS),
                &release::LOAD_SOURCE,
                release::NAME,
                &self.esps,
                self.booted,
            );
            match order {
                Ok(order) => {
                    let list: Vec<String> = order.iter().map(|n| format!("{n:04X}")).collect();
                    post(
                        &refcode::SLOT_CONFIRMED,
                        &format!(
                            "slot {} is the default; BootOrder {}",
                            letter(self.booted),
                            list.join(",")
                        ),
                    );
                }
                Err(e) => {
                    post(
                        &refcode::SLOT_CONFIRMED,
                        &format!("slot {} is the default in the record", letter(self.booted)),
                    );
                    post(&refcode::BOOT_ORDER_NOT_WRITTEN, &e);
                }
            }
        }

        /// The record in words, or that there is none.
        pub fn summary(&self) -> String {
            self.record
                .map_or_else(|| "no A/B record".to_string(), |r| r.summary())
        }
    }
}
