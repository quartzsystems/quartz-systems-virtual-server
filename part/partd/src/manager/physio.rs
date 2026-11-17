// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Physical I/O (`docs/PHYSICALIO.md`): the host's devices as they are,
//! released and taken back at the host's console, given to a partition
//! at its activation with their whole groups, reset and given back at
//! its end, fenced when a reset fails. What can be released and given
//! is `pio`'s arithmetic; what the kernel is asked is the machine's.

use pio::device::Facts;
use pio::{Device, Holder, HostUses, ReleasedDevice};
use qvsmsg::Issued;
use refcode::about;
use state::doc::inventory::{Iommu, Pci};

use super::{Manager, System};
use crate::proto::{DeviceChange, Event, Refused};

/// What physical I/O is decided by, as the machine finds it now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhysioFacts {
    /// The PCI devices.
    pub pci: Vec<Pci>,
    /// The IOMMU.
    pub iommu: Iommu,
    /// What the host uses.
    pub uses: HostUses,
}

/// A device given at an activation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Given {
    /// Its location code.
    pub location: String,
    /// Its PCI address.
    pub address: String,
    /// Its isolation group.
    pub group: u32,
}

fn one(message: Issued) -> Refused {
    vec![message]
}

impl<S: System> Manager<S> {
    /// Who holds what: each running partition's devices.
    fn holders(&self) -> Vec<(String, Holder)> {
        self.partitions
            .values()
            .filter_map(|p| p.run.as_ref().map(|r| (p, r)))
            .flat_map(|(p, r)| {
                r.devices.iter().map(move |l| {
                    (
                        l.clone(),
                        Holder {
                            partition: p.definition.id,
                            name: p.definition.name.clone(),
                            profile: r.profile.clone(),
                        },
                    )
                })
            })
            .collect()
    }

    /// Every device, as the machine finds them now.
    pub(crate) fn device_list(&mut self) -> Vec<Device> {
        let facts = self.system.physio_facts();
        let holders = self.holders();
        pio::assess(&Facts {
            pci: &facts.pci,
            iommu: &facts.iommu,
            uses: &facts.uses,
            released: &self.released,
            holders: &holders,
            fenced: &self.fenced,
        })
    }

    fn device_event(
        &mut self,
        location: &str,
        change: DeviceChange,
        partition: u32,
        note: Option<Issued>,
    ) {
        let time = self.system.now();
        let (name, profile) = self
            .partitions
            .get(&partition)
            .map(|p| {
                let profile = p
                    .run
                    .as_ref()
                    .map_or_else(|| p.last_profile.clone(), |r| r.profile.clone());
                (p.definition.name.clone(), profile)
            })
            .unwrap_or_default();
        let device = self
            .device_list()
            .into_iter()
            .find(|d| d.location == location && d.present);
        self.publish(&Event::Device {
            location: location.to_string(),
            change,
            partition,
            name,
            profile,
            device,
            note,
            time,
        });
    }

    /// Release the device at `location` and the rest of its group for
    /// partitions; `confirm` is the location typed in full. The
    /// locations released.
    pub(crate) fn release_device(
        &mut self,
        location: &str,
        confirm: &str,
    ) -> Result<Vec<String>, Refused> {
        let devices = self.device_list();
        if !devices.iter().any(|d| d.location == location) && self.is_drive(location) {
            return self.release_drive(location, confirm);
        }
        let d = devices
            .iter()
            .find(|d| d.location == location && d.present)
            .ok_or_else(|| one(qvsmsg::DEVICE_UNKNOWN.with(&[location])))?;
        if d.released {
            return Err(one(qvsmsg::DEVICE_RELEASED_ALREADY.with(&[location])));
        }
        if !d.releasable.is_empty() {
            return Err(d.releasable.clone());
        }
        if confirm != location {
            return Err(one(qvsmsg::DEVICE_CONFIRM.with(&[confirm])));
        }
        let group = d.group_locations();
        let members: Vec<&Device> = group
            .iter()
            .filter_map(|l| devices.iter().find(|d| d.location == *l))
            .collect();
        let mut bound: Vec<&Device> = Vec::new();
        for m in &members {
            if let Err(e) = self.system.bind(&m.address, true) {
                for b in bound {
                    let _ = self.system.bind(&b.address, false);
                }
                return Err(one(qvsmsg::DEVICE_NOT_BOUND.with(&[
                    &m.location,
                    "the pass-through driver",
                    &e,
                ])));
            }
            bound.push(m);
        }
        let facts = self.system.physio_facts();
        for l in &group {
            if let Some(p) = facts.pci.iter().find(|p| p.location == *l) {
                self.released.devices.push(ReleasedDevice::of(p, &group));
            }
        }
        if let Err(e) = self.store.save_released(&self.released) {
            self.released
                .devices
                .retain(|r| !group.contains(&r.location));
            for b in &members {
                let _ = self.system.bind(&b.address, false);
            }
            return Err(one(qvsmsg::STATE_NOT_KEPT.with(&[&e])));
        }
        let code = &refcode::DEVICE_RELEASED;
        let abouts: Vec<_> = group.iter().map(|l| about::device(l)).collect();
        self.system
            .post_about(code.code, &code.text_with(&group.join(", ")), &abouts);
        for l in &group {
            self.device_event(l, DeviceChange::Released, 0, None);
        }
        Ok(group)
    }

    /// The profiles that name any of `locations`: each as `PROD
    /// profile NORMAL`.
    fn naming(&self, locations: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for p in self.partitions.values() {
            for (name, profile) in &p.profiles {
                if profile
                    .physical_io
                    .iter()
                    .any(|io| locations.contains(&io.location))
                {
                    out.push(format!("{} profile {name}", p.definition.name));
                }
            }
        }
        out
    }

    /// Take the device at `location` and its group back for the host.
    /// The locations taken back.
    pub(crate) fn take_back_device(&mut self, location: &str) -> Result<Vec<String>, Refused> {
        if self.released.has_drive(location) {
            return self.take_back_drive(location);
        }
        let Some(kept) = self
            .released
            .devices
            .iter()
            .find(|r| r.location == location)
        else {
            return Err(one(qvsmsg::DEVICE_NOT_RELEASED.with(&[location])));
        };
        let mut group = kept.group.clone();
        if !group.iter().any(|l| l == location) {
            group.push(location.to_string());
        }
        if let Some((l, h)) = self.holders().into_iter().find(|(l, _)| group.contains(l)) {
            return Err(one(qvsmsg::DEVICE_HELD.with(&[&l, &h.name])));
        }
        let named = self.naming(&group);
        if !named.is_empty() {
            return Err(one(
                qvsmsg::DEVICE_NAMED.with(&[location, &named.join(", ")])
            ));
        }
        let facts = self.system.physio_facts();
        for l in &group {
            if let Some(p) = facts.pci.iter().find(|p| p.location == *l)
                && let Err(e) = self.system.bind(&p.address, false)
            {
                self.system
                    .log(&format!("{l}: back to the host's driver: {e}"));
            }
        }
        self.released
            .devices
            .retain(|r| !group.contains(&r.location));
        self.fenced.retain(|l| !group.contains(l));
        self.system.keep_fenced(&self.fenced);
        if let Err(e) = self.store.save_released(&self.released) {
            return Err(one(qvsmsg::STATE_NOT_KEPT.with(&[&e])));
        }
        let code = &refcode::DEVICE_TAKEN_BACK;
        let abouts: Vec<_> = group.iter().map(|l| about::device(l)).collect();
        self.system
            .post_about(code.code, &code.text_with(&group.join(", ")), &abouts);
        for l in &group {
            self.device_event(l, DeviceChange::TakenBack, 0, None);
        }
        Ok(group)
    }

    /// Fence `location` after a failed reset, with `partition` the one
    /// that had it or was to.
    fn fence(&mut self, location: &str, partition: u32, why: &str) {
        if !self.fenced.iter().any(|l| l == location) {
            self.fenced.push(location.to_string());
            self.system.keep_fenced(&self.fenced);
        }
        let name = self
            .partitions
            .get(&partition)
            .map(|p| p.definition.name.clone())
            .unwrap_or_default();
        let code = &refcode::DEVICE_FENCED;
        self.system.post_about(
            code.code,
            &code.text_with(&format!("{location}, partition {name}: {why}")),
            &[about::device(location)],
        );
        let note = qvsmsg::DEVICE_FENCED.with(&[location]);
        self.device_event(location, DeviceChange::ResetFailed, partition, Some(note));
    }

    /// What the activation of `id` (`name`) with `wanted` is given, each
    /// device reset first; a device that does not reset is fenced, and
    /// fails the activation when the profile requires it. Why each
    /// device not required was left out.
    pub(crate) fn assign_devices(
        &mut self,
        id: u32,
        name: &str,
        wanted: &[model::profile::PhysicalIo],
    ) -> Result<(Vec<Given>, Vec<Issued>), Refused> {
        if wanted.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let devices = self.device_list();
        let assignment = pio::assignment(&devices, wanted, id, name)?;
        let mut left_out = assignment.left_out;
        let mut given = Vec::new();
        let mut refused = Vec::new();
        // Group by group: a device that does not reset takes its group.
        let mut groups: Vec<Vec<&Device>> = Vec::new();
        for l in &assignment.given {
            let Some(d) = devices.iter().find(|d| d.location == *l) else {
                continue;
            };
            match groups.iter_mut().find(|g| g[0].group == d.group) {
                Some(g) => g.push(d),
                None => groups.push(vec![d]),
            }
        }
        for g in groups {
            let failed: Vec<(String, String)> = g
                .iter()
                .filter_map(|d| {
                    self.system
                        .reset(&d.address)
                        .err()
                        .map(|e| (d.location.clone(), e))
                })
                .collect();
            if failed.is_empty() {
                given.extend(g.iter().map(|d| Given {
                    location: d.location.clone(),
                    address: d.address.clone(),
                    group: d.group.unwrap_or(0),
                }));
                continue;
            }
            for (l, e) in &failed {
                self.fence(l, id, &format!("before it was given: {e}"));
            }
            let required = g.iter().any(|d| {
                wanted
                    .iter()
                    .any(|w| w.location == d.location && w.required)
            });
            for (l, _) in failed {
                let why = qvsmsg::DEVICE_FENCED.with(&[&l]);
                if required {
                    refused.push(qvsmsg::DEVICE_NOT_ASSIGNABLE.with(&[&l, &why.text]));
                } else {
                    left_out.push(qvsmsg::DEVICE_LEFT_OUT.with(&[&l, name, &why.text]));
                }
            }
        }
        if !refused.is_empty() {
            return Err(refused);
        }
        Ok((given, left_out))
    }

    /// The devices are the partition's now: said and published.
    pub(crate) fn assigned(&mut self, id: u32, name: &str, given: &[Given]) {
        if given.is_empty() {
            return;
        }
        let code = &refcode::DEVICE_ASSIGNED;
        let locations: Vec<&str> = given.iter().map(|g| g.location.as_str()).collect();
        let abouts: Vec<_> = locations.iter().map(|l| about::device(l)).collect();
        self.system.post_about(
            code.code,
            &code.text_with(&format!("{}, partition {name}", locations.join(", "))),
            &abouts,
        );
        for l in locations {
            self.device_event(l, DeviceChange::Assigned, id, None);
        }
    }

    /// The partition `id` ended holding `locations`: each is reset and
    /// free again, or fenced.
    pub(crate) fn return_devices(&mut self, id: u32, name: &str, locations: &[String]) {
        if locations.is_empty() {
            return;
        }
        let facts = self.system.physio_facts();
        let mut back = Vec::new();
        for l in locations {
            let Some(p) = facts.pci.iter().find(|p| p.location == *l) else {
                self.fence(l, id, "it is not on the host to be reset");
                continue;
            };
            match self.system.reset(&p.address) {
                Ok(()) => back.push(l.clone()),
                Err(e) => self.fence(l, id, &format!("after the partition ended: {e}")),
            }
        }
        if back.is_empty() {
            return;
        }
        let code = &refcode::DEVICE_RETURNED;
        let abouts: Vec<_> = back.iter().map(|l| about::device(l)).collect();
        self.system.post_about(
            code.code,
            &code.text_with(&format!("{}, partition {name}", back.join(", "))),
            &abouts,
        );
        for l in back {
            self.device_event(&l, DeviceChange::Returned, id, None);
        }
    }

    /// The profile's physical I/O checked against the host's devices
    /// as a put is: a location no device has, part of a group.
    pub(crate) fn check_physical(&mut self, profile: &model::Profile) -> Vec<Issued> {
        if profile.physical_io.is_empty() {
            return Vec::new();
        }
        let devices = self.device_list();
        pio::check_profile(&devices, &profile.physical_io)
    }
}
