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

pub mod efi;

#[cfg(target_os = "linux")]
pub use on_host::{Slots, at_ipl};

#[cfg(target_os = "linux")]
mod on_host {
    use std::fs::File;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use abrec::{AbError, AbPart, Slot};
    use hostdisk::record::{self, HostRecord, Ipl, letter};
    use refcode::RefCode;

    use super::efi;
    use crate::ipl::mounts::EFIVARS;

    /// How long the boot disk may take to appear: some controllers
    /// are probed after PID 1 has started.
    const DISK_WAIT: Duration = Duration::from_secs(10);

    /// The record and what this IPL is by it.
    pub struct Slots {
        /// The slot this IPL came from.
        pub booted: Slot,
        /// The record as it stands, when it could be read or made.
        pub record: Option<HostRecord>,
        /// What this IPL is.
        pub ipl: Ipl,
        part: Option<AbPart<File>>,
    }

    /// The record partition, waited for.
    fn open() -> Result<AbPart<File>, AbError> {
        let give_up = Instant::now() + DISK_WAIT;
        loop {
            match record::open() {
                Err(AbError::NotFound) if Instant::now() < give_up => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                other => return other,
            }
        }
    }

    /// The record step of an IPL from `booted`: read the record, note
    /// what this IPL is, and write the record back when that changes
    /// it. `post` is given each code with its detail.
    pub fn at_ipl(booted: Slot, mut post: impl FnMut(&RefCode, &str)) -> Slots {
        let mut slots = Slots {
            booted,
            record: None,
            ipl: Ipl::Normal,
            part: None,
        };
        let mut part = match open() {
            Ok(part) => part,
            Err(e) => {
                post(&refcode::RECORD_INVALID, &format!("not read: {e}"));
                return slots;
            }
        };
        let read = record::read(&mut part);
        let (ipl, now) = match &read {
            Ok((found, _)) => found.at_ipl(booted),
            // No copy is a record: start one from the slot that booted.
            Err(_) => (Ipl::Normal, HostRecord::new(booted)),
        };
        let written = match &read {
            Ok((found, _)) if *found == now => Ok(()),
            _ => record::write(&mut part, now).map(|_| ()),
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
                if let Err(e) = record::write(part, next) {
                    post(&refcode::RECORD_INVALID, &format!("not written: {e}"));
                    return;
                }
                self.record = Some(next);
            }
            if !trial {
                return;
            }
            self.ipl = Ipl::Normal;
            let order = hostdisk::locate::esp()
                .and_then(|(_, esp)| efi::make_default(Path::new(EFIVARS), &esp, self.booted));
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
