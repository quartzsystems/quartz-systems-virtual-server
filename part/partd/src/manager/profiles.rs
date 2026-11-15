// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Revisions of definitions and profiles, so two consoles cannot both
//! write what one of them read: a put or a change made at a revision
//! is refused when the revision is not the one kept. Saving an
//! activated partition's current configuration as a profile.

use model::profile::{ProcessorMode, Sharing};
use model::{Definition, Profile};
use resource::{Current, Processors};

use super::{Manager, System};
use crate::proto::{Event, ProfileEntry, Refused};

fn one(issued: qvsmsg::Issued) -> Refused {
    vec![issued]
}

/// A profile as `current` ran it: the profile the partition was
/// activated with, its desired processors and memory what the
/// activation gave, its minimum and maximum widened to take them.
pub(crate) fn as_current(profile: &Profile, name: &str, current: &Current) -> Profile {
    let mut saved = profile.clone();
    saved.name = name.to_string();
    let p = &mut saved.processors;
    match &current.processors {
        Processors::Dedicated { cores } => {
            let n = u32::try_from(cores.len()).unwrap_or(u32::MAX);
            p.mode = ProcessorMode::Dedicated;
            p.desired = n;
            p.minimum = p.minimum.min(n);
            p.maximum = p.maximum.max(n);
            p.minimum_units = None;
            p.desired_units = None;
            p.maximum_units = None;
            p.sharing = None;
            p.weight = None;
            p.pool = None;
        }
        Processors::Shared {
            pool,
            units,
            virtuals,
            capped,
            weight,
        } => {
            p.mode = ProcessorMode::Shared;
            p.desired = *virtuals;
            p.minimum = p.minimum.min(*virtuals);
            p.maximum = p.maximum.max(*virtuals);
            p.desired_units = Some(*units);
            p.minimum_units = Some(p.minimum_units.map_or(*units, |m| m.min(*units)));
            p.maximum_units = Some(p.maximum_units.map_or(*units, |m| m.max(*units)));
            p.sharing = Some(if *capped {
                Sharing::Capped
            } else {
                Sharing::Uncapped
            });
            p.weight = (!*capped).then_some(*weight);
            p.pool = Some(pool.clone());
        }
    }
    let m = &mut saved.memory;
    m.desired_mib = current.memory_mib;
    m.minimum_mib = m.minimum_mib.min(current.memory_mib);
    m.maximum_mib = m.maximum_mib.max(current.memory_mib);
    saved
}

impl<S: System> Manager<S> {
    /// Say a profile of `id` changed: its revision now, none when it
    /// is gone.
    pub(crate) fn profile_changed(&mut self, id: u32, profile: &str) {
        let time = self.system.now();
        let Some(p) = self.partitions.get(&id) else {
            return;
        };
        let event = Event::Profile {
            id,
            name: p.definition.name.clone(),
            profile: profile.to_string(),
            revision: p.profiles.get(profile).map(|p| p.revision),
            default: p.definition.default_profile == profile,
            time,
        };
        self.publish(&event);
    }

    /// Refused unless the profile `name` of `id` is at `revision`
    /// (`None`: there is none of that name).
    pub(crate) fn profile_at(
        &self,
        id: u32,
        name: &str,
        revision: Option<u64>,
    ) -> Result<(), Refused> {
        let p = self.partition(id)?;
        let partition = &p.definition.name;
        match (p.profiles.get(name), revision) {
            (None, None) => Ok(()),
            (Some(_), None) => Err(one(qvsmsg::PROFILE_EXISTS.with(&[partition, name]))),
            (Some(kept), Some(r)) if kept.revision == r => Ok(()),
            (_, Some(r)) => Err(one(qvsmsg::PROFILE_CHANGED.with(&[
                partition,
                name,
                &r.to_string(),
            ]))),
        }
    }

    /// The profile `name` of `id` from its current configuration.
    pub(crate) fn current_as(&self, id: u32, name: &str) -> Result<Profile, Refused> {
        let p = self.partition(id)?;
        let partition = p.definition.name.clone();
        let Some(run) = p.run.as_ref() else {
            return Err(one(qvsmsg::NOTHING_TO_SAVE.with(&[&partition])));
        };
        let profile = p
            .profiles
            .get(&run.profile)
            .ok_or_else(|| one(qvsmsg::PROFILE_MISSING.with(&[&partition, &run.profile])))?;
        Ok(as_current(profile, name, &run.current))
    }

    /// Refused unless the definition of `definition.id` is at
    /// `revision`.
    pub(crate) fn definition_at(
        &self,
        definition: &Definition,
        revision: u64,
    ) -> Result<(), Refused> {
        let p = self.partition(definition.id)?;
        if p.definition.revision == revision {
            Ok(())
        } else {
            Err(one(
                qvsmsg::DEFINITION_CHANGED.with(&[&p.definition.name, &revision.to_string()])
            ))
        }
    }

    /// The profiles of `id` with their revisions.
    pub(crate) fn entries(&self, id: u32) -> Result<Vec<ProfileEntry>, Refused> {
        let p = self.partition(id)?;
        Ok(p.profiles
            .values()
            .map(|profile| ProfileEntry {
                name: profile.name.clone(),
                revision: profile.revision,
                default: p.definition.default_profile == profile.name,
            })
            .collect())
    }
}
