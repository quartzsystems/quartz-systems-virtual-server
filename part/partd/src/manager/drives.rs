// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Physical tape drives passed through (`docs/STORAGE.md`): a drive on
//! a controller the host keeps, released at the host like a device, is
//! named by a tape device of a profile as `@` and its location code, and
//! is one partition's at a time. At the activation the machine opens the
//! drive and a relay to it, whose far end the VMM is given; the relay
//! checks every command against the allow-list. A drive's cartridge is
//! changed at the drive, not loaded or unloaded from a console.

use model::Profile;
use model::validate::tape_drive;
use qvsmsg::Issued;
use refcode::about;

use super::{Manager, System};
use crate::proto::{Event, Refused, TapeDrive};

/// A drive as the machine finds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriveFacts {
    /// Its location code.
    pub location: String,
    /// Its vendor.
    pub vendor: String,
    /// Its product.
    pub product: String,
    /// Its serial number.
    pub serial: String,
    /// Its SCSI generic device: `sg2`.
    pub sg: String,
    /// Whether a cartridge is in it: asked of a drive nobody holds;
    /// none for one held.
    pub loaded: Option<bool>,
}

fn one(message: Issued) -> Refused {
    vec![message]
}

/// The physical drives a profile's tape devices name: each slot and the
/// drive's location code.
pub(crate) fn named_drives(profile: &Profile) -> Vec<(u32, String)> {
    profile
        .tape
        .iter()
        .filter_map(|t| Some((t.slot, tape_drive(t.media.as_deref()?)?.to_string())))
        .collect()
}

impl<S: System> Manager<S> {
    /// Who holds each drive: its location, the partition's ID and name,
    /// the slot.
    fn drive_holders(&self) -> Vec<(String, u32, String, u32)> {
        self.partitions
            .values()
            .filter_map(|p| p.run.as_ref().map(|r| (p, r)))
            .flat_map(|(p, r)| {
                r.drives.iter().map(move |(slot, l)| {
                    (l.clone(), p.definition.id, p.definition.name.clone(), *slot)
                })
            })
            .collect()
    }

    /// The host's physical tape drives, and every released one that is
    /// not on it now.
    pub(crate) fn drive_list(&mut self) -> Vec<TapeDrive> {
        let holders = self.drive_holders();
        let held: Vec<String> = holders.iter().map(|(l, ..)| l.clone()).collect();
        let facts = self.system.tape_drives(&held);
        let mut out: Vec<TapeDrive> = facts
            .iter()
            .map(|f| {
                let holder = holders.iter().find(|(l, ..)| *l == f.location);
                TapeDrive {
                    location: f.location.clone(),
                    vendor: f.vendor.clone(),
                    product: f.product.clone(),
                    serial: f.serial.clone(),
                    present: true,
                    released: self.released.has_drive(&f.location),
                    loaded: f.loaded.unwrap_or(false),
                    holder: holder.map_or(0, |h| h.1),
                    name: holder.map(|h| h.2.clone()).unwrap_or_default(),
                    slot: holder.map_or(0, |h| h.3),
                }
            })
            .collect();
        for r in &self.released.drives {
            if !out.iter().any(|d| d.location == r.location) {
                out.push(TapeDrive {
                    location: r.location.clone(),
                    serial: r.serial.clone(),
                    released: true,
                    ..TapeDrive::default()
                });
            }
        }
        out.sort_by(|a, b| a.location.cmp(&b.location));
        out
    }

    fn drive_event(&mut self, location: &str) {
        let time = self.system.now();
        let drive = self
            .drive_list()
            .into_iter()
            .find(|d| d.location == location && d.present);
        self.publish(&Event::TapeDrive {
            location: location.to_string(),
            drive,
            time,
        });
    }

    /// Whether `location` is a physical tape drive of the host.
    pub(crate) fn is_drive(&mut self, location: &str) -> bool {
        self.drive_list()
            .iter()
            .any(|d| d.location == location && d.present)
    }

    /// Release the drive at `location`, typed in full.
    pub(crate) fn release_drive(
        &mut self,
        location: &str,
        confirm: &str,
    ) -> Result<Vec<String>, Refused> {
        let drives = self.drive_list();
        let d = drives
            .iter()
            .find(|d| d.location == location && d.present)
            .ok_or_else(|| one(qvsmsg::NOT_A_DRIVE.with(&[location])))?;
        if d.released {
            return Err(one(qvsmsg::DEVICE_RELEASED_ALREADY.with(&[location])));
        }
        if confirm != location {
            return Err(one(qvsmsg::DEVICE_CONFIRM.with(&[confirm])));
        }
        self.released.drives.push(pio::released::ReleasedDrive {
            location: location.to_string(),
            serial: d.serial.clone(),
        });
        if let Err(e) = self.store.save_released(&self.released) {
            self.released.drives.retain(|r| r.location != location);
            return Err(one(qvsmsg::STATE_NOT_KEPT.with(&[&e])));
        }
        let code = &refcode::DEVICE_RELEASED;
        self.system.post_about(
            code.code,
            &code.text_with(location),
            &[about::tape_drive(location, &d.serial)],
        );
        self.drive_event(location);
        Ok(vec![location.to_string()])
    }

    /// Take the drive at `location` back, when no profile names it and
    /// no partition holds it.
    pub(crate) fn take_back_drive(&mut self, location: &str) -> Result<Vec<String>, Refused> {
        if !self.released.has_drive(location) {
            return Err(one(qvsmsg::DEVICE_NOT_RELEASED.with(&[location])));
        }
        if let Some((_, _, name, _)) = self.drive_holders().into_iter().find(|h| h.0 == location) {
            return Err(one(qvsmsg::DEVICE_HELD.with(&[location, &name])));
        }
        let mut named = Vec::new();
        for p in self.partitions.values() {
            for (pname, profile) in &p.profiles {
                if named_drives(profile).iter().any(|(_, l)| l == location) {
                    named.push(format!("{} profile {pname}", p.definition.name));
                }
            }
        }
        if !named.is_empty() {
            return Err(one(
                qvsmsg::DEVICE_NAMED.with(&[location, &named.join(", ")])
            ));
        }
        self.released.drives.retain(|r| r.location != location);
        if let Err(e) = self.store.save_released(&self.released) {
            return Err(one(qvsmsg::STATE_NOT_KEPT.with(&[&e])));
        }
        let code = &refcode::DEVICE_TAKEN_BACK;
        self.system.post_about(
            code.code,
            &code.text_with(location),
            &[about::tape_drive(location, "")],
        );
        self.drive_event(location);
        Ok(vec![location.to_string()])
    }

    /// What a put of `profile` is refused for: a tape device that names
    /// what is not a tape drive of the host.
    pub(crate) fn check_drives(&mut self, profile: &Profile) -> Vec<Issued> {
        let named = named_drives(profile);
        if named.is_empty() {
            return Vec::new();
        }
        let drives = self.drive_list();
        named
            .iter()
            .filter(|(_, l)| !drives.iter().any(|d| d.location == *l))
            .map(|(_, l)| qvsmsg::NOT_A_DRIVE.with(&[l]))
            .collect()
    }

    /// The drives the activation of `id` (`name`) with `profile` holds,
    /// each opened with its relay: the VMM's descriptors from `first` in
    /// order. Each slot with its drive's relay and location.
    pub(crate) fn drives_for(
        &mut self,
        id: u32,
        name: &str,
        profile: &Profile,
        first: usize,
    ) -> Result<Vec<(u32, String, String)>, Refused> {
        let named = named_drives(profile);
        if named.is_empty() {
            return Ok(Vec::new());
        }
        let holders = self.drive_holders();
        let held: Vec<String> = holders.iter().map(|(l, ..)| l.clone()).collect();
        let facts = self.system.tape_drives(&held);
        let mut errors = Vec::new();
        for (_, location) in &named {
            if let Some((_, other, holder, _)) = holders.iter().find(|h| h.0 == *location)
                && *other != id
            {
                errors.push(qvsmsg::DEVICE_HELD.with(&[location, holder]));
            } else if !self.released.has_drive(location) {
                errors.push(qvsmsg::DEVICE_NOT_RELEASED.with(&[location]));
            } else if !facts.iter().any(|f| f.location == *location) {
                errors.push(qvsmsg::DRIVE_GONE.with(&[location]));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let mut out = Vec::new();
        for (i, (slot, location)) in named.iter().enumerate() {
            let sg = facts
                .iter()
                .find(|f| f.location == *location)
                .map(|f| f.sg.clone())
                .unwrap_or_default();
            match self.system.open_drive(id, location, &sg, first + i) {
                Ok(path) => out.push((*slot, path, location.clone())),
                Err(e) => {
                    return Err(one(qvsmsg::DEVICE_NOT_OPENED.with(&[location, name, &e])));
                }
            }
        }
        Ok(out)
    }

    /// The drives of `id` are its now, or free again: published.
    pub(crate) fn drives_moved(&mut self, drives: &[(u32, String)]) {
        for (_, location) in drives {
            self.drive_event(location);
        }
    }
}
