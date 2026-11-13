// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The socket's requests, each against the partitions as they are. A
//! request that is refused changes nothing.

use model::definition::{MAX_ID, serial_for};
use model::profile::DEFAULT_POOL;
use model::{AutoStart, Definition, Environment, IplSource, Keylock, Limits, Profile};
use qvsmsg::Issued;
use resource::{Current, HostResources, Pool, Processors, Settings};

use super::{Manager, Partition, System};
use crate::proto::{Event, Partitions, Posted, Refused, Status};
use crate::store::{Kept, MAX_PROFILES};

fn one(message: Issued) -> Refused {
    vec![message]
}

impl<S: System> Manager<S> {
    /// What validation takes of the settings.
    pub(crate) fn limits(&self) -> Limits {
        Limits {
            memory_block_mib: self.host.settings.memory_block_mib,
        }
    }

    fn partition(&self, id: u32) -> Result<&Partition, Refused> {
        self.partitions
            .get(&id)
            .ok_or_else(|| one(qvsmsg::PARTITION_MISSING.with(&[&id.to_string()])))
    }

    /// The partition `id`, which must not be activated.
    fn idle(&self, id: u32) -> Result<&Partition, Refused> {
        let partition = self.partition(id)?;
        if partition.state.active() {
            return Err(one(qvsmsg::PARTITION_ACTIVE
                .with(&[&partition.definition.name, partition.state.word()])));
        }
        Ok(partition)
    }

    fn defined(&mut self, id: u32, name: &str, exists: bool) {
        let time = self.system.now();
        self.publish(&Event::Defined {
            id,
            name: name.to_string(),
            exists,
            time,
        });
    }

    fn keep_definition(&mut self, definition: &Definition) -> Result<(), Refused> {
        self.store
            .save_definition(definition)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The partition's definition", &e])))
    }

    /// The settings with `pools` in place of theirs, when no pool is
    /// in error against the host and what runs.
    fn with_pools(&self, pools: Vec<Pool>) -> Result<Settings, Refused> {
        let mut host = self.host.clone();
        host.settings.pools = pools;
        let held = self.held();
        let mut errors = host.pool_errors(&held);
        // The default pool keeps what its running partitions need: a
        // core for each virtual processor, and their units.
        if let Some(default) = host.pool(DEFAULT_POOL, &held) {
            let widest = held
                .iter()
                .filter_map(|h| match &h.current.processors {
                    Processors::Shared { pool, virtuals, .. } if pool == DEFAULT_POOL => {
                        Some(*virtuals)
                    }
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            let cores = u32::try_from(default.cores.len()).unwrap_or(u32::MAX);
            if default.entitled > default.maximum || widest > cores {
                errors.push(qvsmsg::POOL_CORES_NEEDED.with(&[
                    &default.entitled.to_string(),
                    &widest.to_string(),
                    &cores.to_string(),
                ]));
            }
        }
        if errors.is_empty() {
            Ok(host.settings)
        } else {
            Err(errors)
        }
    }

    fn keep_settings(&mut self, settings: Settings) -> Result<(), Refused> {
        self.store
            .save_settings(&settings)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The host's settings", &e])))?;
        self.host.settings = settings;
        if let Err(e) = self.apply_base() {
            self.system.log(&format!("resource controls: {e}"));
        }
        Ok(())
    }

    /// How many partitions of the pool `name` run.
    fn running_in(&self, name: &str) -> usize {
        self.held()
            .iter()
            .filter(|h| {
                matches!(&h.current.processors, Processors::Shared { pool, .. } if pool == name)
            })
            .count()
    }

    /// The pool `name`, which must be a named one with nothing running
    /// in it: its place among the pools.
    fn idle_pool(&self, name: &str) -> Result<usize, Refused> {
        if name == DEFAULT_POOL {
            return Err(one(qvsmsg::POOL_DEFAULT.issue()));
        }
        let at = self
            .host
            .settings
            .pools
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| one(qvsmsg::POOL_MISSING.with(&[name])))?;
        let running = self.running_in(name);
        if running > 0 {
            return Err(one(qvsmsg::POOL_IN_USE.with(&[name, &running.to_string()])));
        }
        Ok(at)
    }

    /// The partitions flagged to start with the host, in their order.
    pub fn auto_start(&self) -> Vec<(u32, String, u32)> {
        let mut flagged: Vec<&Definition> = self
            .partitions
            .values()
            .map(|p| &p.definition)
            .filter(|d| d.auto_start.enabled)
            .collect();
        flagged.sort_by_key(|d| (d.auto_start.order, d.id));
        flagged
            .into_iter()
            .map(|d| (d.id, d.name.clone(), d.auto_start.delay_secs))
            .collect()
    }
}

impl<S: System> Partitions for Manager<S> {
    fn list(&mut self) -> Vec<Status> {
        self.statuses()
    }

    fn create(&mut self, name: String) -> Result<Definition, Refused> {
        if !model::valid_name(&name) {
            return Err(one(qvsmsg::NAME_INVALID.with(&[&name])));
        }
        if let Some(other) = self.partitions.values().find(|p| p.definition.name == name) {
            return Err(one(
                qvsmsg::NAME_IN_USE.with(&[&name, &other.definition.id.to_string()])
            ));
        }
        let most = self.host.settings.max_partitions;
        if self.partitions.len() >= most as usize {
            return Err(one(qvsmsg::TOO_MANY_PARTITIONS.with(&[&most.to_string()])));
        }
        let id = (1..=MAX_ID)
            .find(|id| !self.partitions.contains_key(id))
            .ok_or_else(|| one(qvsmsg::TOO_MANY_PARTITIONS.with(&[&MAX_ID.to_string()])))?;
        let definition = Definition {
            id,
            name: name.clone(),
            environment: Environment::Qs400,
            serial: serial_for(&self.host_serial, id),
            keylock: Keylock::Normal,
            default_profile: String::new(),
            auto_start: AutoStart::default(),
            created: self.system.now(),
        };
        self.keep_definition(&definition)?;
        let kept = Kept {
            definition: definition.clone(),
            profiles: std::collections::BTreeMap::new(),
        };
        self.partitions.insert(id, Partition::of(kept));
        self.defined(id, &name, true);
        Ok(definition)
    }

    fn definition(&mut self, id: u32) -> Result<Definition, Refused> {
        self.partition(id).map(|p| p.definition.clone())
    }

    fn change(&mut self, definition: Definition) -> Result<(), Refused> {
        let id = definition.id;
        let partition = self.idle(id)?;
        let old = &partition.definition;
        let mut errors = model::validate::definition(&definition);
        for (field, same) in [
            ("serial number", old.serial == definition.serial),
            ("environment", old.environment == definition.environment),
            ("time of creation", old.created == definition.created),
        ] {
            if !same {
                errors.push(qvsmsg::FIELD_FIXED.with(&[field, &old.name]));
            }
        }
        if let Some(other) = self
            .partitions
            .values()
            .find(|p| p.definition.id != id && p.definition.name == definition.name)
        {
            errors.push(
                qvsmsg::NAME_IN_USE.with(&[&definition.name, &other.definition.id.to_string()]),
            );
        }
        let profile = &definition.default_profile;
        if !profile.is_empty() && !partition.profiles.contains_key(profile) {
            errors.push(qvsmsg::DEFAULT_PROFILE_MISSING.with(&[&definition.name, profile]));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        self.keep_definition(&definition)?;
        let name = definition.name.clone();
        if let Some(partition) = self.partitions.get_mut(&id) {
            partition.definition = definition;
        }
        self.defined(id, &name, true);
        Ok(())
    }

    fn delete(&mut self, id: u32) -> Result<(), Refused> {
        let name = self.idle(id)?.definition.name.clone();
        self.store
            .delete(id)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The partition's deletion", &e])))?;
        self.partitions.remove(&id);
        self.defined(id, &name, false);
        Ok(())
    }

    fn profiles(&mut self, id: u32) -> Result<Vec<String>, Refused> {
        self.partition(id)
            .map(|p| p.profiles.keys().cloned().collect())
    }

    fn profile(&mut self, id: u32, name: String) -> Result<Profile, Refused> {
        let partition = self.partition(id)?;
        partition
            .profiles
            .get(&name)
            .cloned()
            .ok_or_else(|| one(qvsmsg::PROFILE_MISSING.with(&[&partition.definition.name, &name])))
    }

    fn put_profile(&mut self, id: u32, profile: Profile) -> Result<(), Refused> {
        let partition = self.partition(id)?;
        let errors = model::validate::profile(&profile, self.limits());
        if !errors.is_empty() {
            return Err(errors);
        }
        let new = !partition.profiles.contains_key(&profile.name);
        if new && partition.profiles.len() >= MAX_PROFILES {
            return Err(one(qvsmsg::TOO_MANY_PROFILES
                .with(&[&partition.definition.name, &MAX_PROFILES.to_string()])));
        }
        self.store
            .save_profile(id, &profile)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The profile", &e])))?;
        // The first profile of a partition is its default.
        let mut definition = partition.definition.clone();
        let first = definition.default_profile.is_empty();
        if first {
            definition.default_profile.clone_from(&profile.name);
            self.keep_definition(&definition)?;
        }
        let name = definition.name.clone();
        if let Some(partition) = self.partitions.get_mut(&id) {
            partition.profiles.insert(profile.name.clone(), profile);
            partition.definition = definition;
        }
        self.defined(id, &name, true);
        Ok(())
    }

    fn delete_profile(&mut self, id: u32, name: String) -> Result<(), Refused> {
        let partition = self.partition(id)?;
        let partition_name = partition.definition.name.clone();
        if !partition.profiles.contains_key(&name) {
            return Err(one(qvsmsg::PROFILE_MISSING.with(&[&partition_name, &name])));
        }
        if partition.definition.default_profile == name {
            return Err(one(
                qvsmsg::PROFILE_IS_DEFAULT.with(&[&partition_name, &name])
            ));
        }
        self.store
            .delete_profile(id, &name)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The profile's deletion", &e])))?;
        if let Some(partition) = self.partitions.get_mut(&id) {
            partition.profiles.remove(&name);
        }
        self.defined(id, &partition_name, true);
        Ok(())
    }

    fn activate(
        &mut self,
        id: u32,
        profile: String,
        source: Option<IplSource>,
        keylock: Option<Keylock>,
    ) -> Result<Current, Refused> {
        let result = self
            .resolve(id, &profile, source)
            .and_then(|(profile, current)| {
                self.launch(id, &profile, current.clone(), source, keylock)
                    .map(|()| current)
                    .map_err(one)
            });
        if let Err(errors) = &result {
            // A partition that is activated already is not one whose
            // activation failed.
            let active = self.partitions.get(&id).is_some_and(|p| p.state.active());
            if !active {
                self.not_activated(id, errors);
            }
        }
        result
    }

    fn shut_down(&mut self, id: u32) -> Result<(), Refused> {
        self.end_one(id, false)
    }

    fn restart(&mut self, id: u32) -> Result<(), Refused> {
        self.end_one(id, true)
    }

    fn resources(&mut self) -> HostResources {
        self.host.resources(&self.held())
    }

    fn create_pool(&mut self, pool: Pool) -> Result<(), Refused> {
        if pool.name == DEFAULT_POOL {
            return Err(one(qvsmsg::POOL_DEFAULT.issue()));
        }
        let mut pools = self.host.settings.pools.clone();
        pools.push(pool);
        let settings = self.with_pools(pools)?;
        self.keep_settings(settings)
    }

    fn change_pool(&mut self, pool: Pool) -> Result<(), Refused> {
        let at = self.idle_pool(&pool.name)?;
        let mut pools = self.host.settings.pools.clone();
        pools[at] = pool;
        let settings = self.with_pools(pools)?;
        self.keep_settings(settings)
    }

    fn delete_pool(&mut self, name: String) -> Result<(), Refused> {
        let at = self.idle_pool(&name)?;
        let mut pools = self.host.settings.pools.clone();
        pools.remove(at);
        let settings = self.with_pools(pools)?;
        self.keep_settings(settings)?;
        self.system
            .remove_group(&resource::cgroup::pool_group(&name));
        Ok(())
    }

    fn history(&mut self, id: u32) -> Result<Vec<Posted>, Refused> {
        self.partition(id)
            .map(|p| p.history.iter().cloned().collect())
    }

    fn settings(&mut self) -> Settings {
        self.host.settings.clone()
    }

    fn change_settings(&mut self, settings: Settings) -> Result<(), Refused> {
        // The pools have their own requests.
        let settings = Settings {
            pools: self.host.settings.pools.clone(),
            ..settings
        };
        let mut errors = settings.errors();
        let cores = self.host.topology.core_count();
        if settings.reserved_cores >= cores.max(1) && cores > 0 {
            errors.push(qvsmsg::SETTING_INVALID.with(&[
                "reserved_cores",
                &settings.reserved_cores.to_string(),
                &format!("less than the host's {cores} cores"),
            ]));
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        // The reserve is the next IPL's: what is configurable now is
        // what was set aside at this one.
        let reserved = self.host.settings.reserved_cores;
        self.store
            .save_settings(&settings)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The host's settings", &e])))?;
        self.host.settings = Settings {
            reserved_cores: reserved,
            ..settings
        };
        Ok(())
    }

    fn end_all(&mut self) -> u32 {
        let mut active: Vec<(u64, u32)> = self
            .partitions
            .values()
            .filter(|p| p.state.active())
            .map(|p| (p.run.as_ref().map_or(0, |r| r.sequence), p.definition.id))
            .collect();
        // The last one activated is the first to end.
        active.sort_unstable_by(|a, b| b.cmp(a));
        let queue: Vec<u32> = active.into_iter().map(|(_, id)| id).collect();
        let count = u32::try_from(queue.len()).unwrap_or(u32::MAX);
        let first = queue.first().copied();
        self.ending = Some(queue);
        if let Some(first) = first {
            self.begin_end(first, false);
        }
        count
    }

    fn subscribe(&mut self) {}
}

impl<S: System> Manager<S> {
    fn end_one(&mut self, id: u32, again: bool) -> Result<(), Refused> {
        let partition = self.partition(id)?;
        let name = &partition.definition.name;
        match partition.state {
            crate::proto::State::ShuttingDown => Err(one(qvsmsg::PARTITION_ENDING.with(&[name]))),
            state if !state.active() => {
                Err(one(qvsmsg::PARTITION_NOT_ACTIVE.with(&[name, state.word()])))
            }
            _ => {
                self.begin_end(id, again);
                Ok(())
            }
        }
    }
}
