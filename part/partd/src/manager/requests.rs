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

use model::validate::{Backing, backing};
use poold::proto::Want;

use super::life::media_kind;
use super::{Manager, Partition, System};
use crate::proto::{
    DumpInfo, Event, MediaSlot, PanelArgs, Partitions, Posted, Refused, ScreenCapture, Shutdown,
    State, Status,
};
use crate::store::{Kept, MAX_PROFILES};

fn one(message: Issued) -> Refused {
    vec![message]
}

impl<S: System> Manager<S> {
    /// A running partition's name and what its device in `slot` takes.
    fn media_slot(&self, id: u32, slot: u32) -> Result<(String, poold::proto::Kind), Refused> {
        let partition = self.partition(id)?;
        let name = partition.definition.name.clone();
        if !matches!(partition.state, State::Starting | State::Running) {
            return Err(one(qvsmsg::PARTITION_NOT_ACTIVE.with(&[&name])));
        }
        let kind = partition
            .profiles
            .get(&partition.last_profile)
            .and_then(|p| media_kind(p, slot))
            .ok_or_else(|| one(qvsmsg::MEDIA_SLOT_MISSING.with(&[&name, &slot.to_string()])))?;
        Ok((name, kind))
    }

    /// Record the medium in `slot` of a running partition; what was
    /// there before.
    fn set_medium(&mut self, id: u32, slot: u32, medium: Option<String>) -> Option<String> {
        let run = self.partitions.get_mut(&id)?.run.as_mut()?;
        if let Some((_, m)) = run.media.iter_mut().find(|(s, _)| *s == slot) {
            return std::mem::replace(m, medium);
        }
        run.media.push((slot, medium));
        None
    }

    /// What validation takes of the settings.
    pub(crate) fn limits(&self) -> Limits {
        Limits {
            memory_block_mib: self.host.settings.memory_block_mib,
        }
    }

    /// The host's MAC prefix as kept, or as its serial number makes it.
    pub(crate) fn prefix(&self) -> [u8; 3] {
        self.host
            .settings
            .mac_prefix
            .as_deref()
            .and_then(|p| vswitch::mac::parse(&format!("{p}:00:00:00")))
            .map_or_else(
                || vswitch::mac::prefix_of(&self.host_serial),
                |m| [m[0], m[1], m[2]],
            )
    }

    /// The host's MAC prefix, kept in `partitions.toml` the first time:
    /// it never changes afterwards.
    fn kept_prefix(&mut self) -> Result<[u8; 3], Refused> {
        let prefix = self.prefix();
        if self.host.settings.mac_prefix.is_none() {
            let text =
                vswitch::mac::text(&[prefix[0], prefix[1], prefix[2], 0, 0, 0])[..8].to_string();
            // What the file says, which may be the next IPL's.
            let mut kept = self
                .store
                .settings()
                .unwrap_or_else(|_| self.host.settings.clone());
            kept.mac_prefix = Some(text.clone());
            self.store
                .save_settings(&kept)
                .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The host's settings", &e])))?;
            self.host.settings.mac_prefix = Some(text);
        }
        Ok(prefix)
    }

    /// The errors of `profile`'s adapters for partition `id` that need
    /// the host: an address another partition has, a switch that is not
    /// there, a VLAN it does not carry.
    fn ethernet_errors(&mut self, id: u32, profile: &Profile) -> Vec<Issued> {
        let mut errors = Vec::new();
        for a in &profile.ethernet {
            let Some(mac) = a.mac.as_deref() else {
                continue;
            };
            let other = self.partitions.values().find(|p| {
                p.definition.id != id
                    && p.profiles.values().flat_map(|p| &p.ethernet).any(|o| {
                        o.mac
                            .as_deref()
                            .is_some_and(|m| m.eq_ignore_ascii_case(mac))
                    })
            });
            if let Some(other) = other {
                errors.push(qvsmsg::MAC_IN_USE.with(&[
                    &a.slot.to_string(),
                    mac,
                    &format!("partition {}", other.definition.name),
                ]));
            }
        }
        if profile.ethernet.is_empty() {
            return errors;
        }
        let Some(switches) = self.system.switches() else {
            self.system.log(
                "the switch service did not answer: the profile's switches are checked at its activation",
            );
            return errors;
        };
        for a in &profile.ethernet {
            let Some(name) = a.switch.as_deref() else {
                continue;
            };
            let slot = a.slot.to_string();
            match switches.iter().find(|s| s.name == name) {
                None => errors.push(qvsmsg::ADAPTER_SWITCH_MISSING.with(&[&slot, name])),
                Some(s) => {
                    for v in a.port_vlan().into_iter().chain(a.tagged_vlans()) {
                        if !s.carries(v) {
                            errors.push(qvsmsg::ADAPTER_VLAN_MISSING.with(&[
                                &slot,
                                name,
                                &v.to_string(),
                            ]));
                        }
                    }
                }
            }
        }
        errors
    }

    pub(crate) fn partition(&self, id: u32) -> Result<&Partition, Refused> {
        self.partitions
            .get(&id)
            .ok_or_else(|| one(qvsmsg::PARTITION_MISSING.with(&[&id.to_string()])))
    }

    /// The partition `id`, which must not be activated.
    fn idle(&self, id: u32) -> Result<&Partition, Refused> {
        let partition = self.partition(id)?;
        if partition.active() {
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

    /// How long auto start waits for `id`'s IPL to complete before the
    /// next; none when its definition does not wait.
    pub fn auto_wait(&self, id: u32) -> Option<u32> {
        let auto = self.partitions.get(&id)?.definition.auto_start;
        auto.wait.then_some(auto.wait_secs)
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

    fn delete(&mut self, id: u32, disks: bool) -> Result<(), Refused> {
        let partition = self.idle(id)?;
        let name = partition.definition.name.clone();
        if disks {
            // Its virtual disks go first: when one cannot, nothing does.
            let mut named: Vec<(String, String)> = Vec::new();
            for profile in partition.profiles.values() {
                for (_, unit) in profile.units() {
                    if let Some(Backing::Vdisk { pool, name }) = backing(&unit.backing) {
                        let disk = (pool.to_string(), name.to_string());
                        if !named.contains(&disk) {
                            named.push(disk);
                        }
                    }
                }
            }
            let refused = self.system.delete_vdisks(&named);
            if !refused.is_empty() {
                return Err(refused);
            }
        }
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
        let mut profile = profile;
        let errors = model::validate::profile(&profile, self.limits());
        if !errors.is_empty() {
            return Err(errors);
        }
        let new = !partition.profiles.contains_key(&profile.name);
        if new && partition.profiles.len() >= MAX_PROFILES {
            return Err(one(qvsmsg::TOO_MANY_PROFILES
                .with(&[&partition.definition.name, &MAX_PROFILES.to_string()])));
        }
        let mut definition = partition.definition.clone();
        // An adapter saved without an address is given one, kept from then on.
        if profile.ethernet.iter().any(|a| a.mac.is_none()) {
            let prefix = self.kept_prefix()?;
            for a in &mut profile.ethernet {
                if a.mac.is_none() {
                    a.mac = Some(vswitch::mac::text(&vswitch::mac::made(prefix, id, a.slot)));
                }
            }
        }
        let errors = self.ethernet_errors(id, &profile);
        if !errors.is_empty() {
            return Err(errors);
        }
        self.store
            .save_profile(id, &profile)
            .map_err(|e| one(qvsmsg::STATE_NOT_KEPT.with(&["The profile", &e])))?;
        // The first profile of a partition is its default.
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
        // Function 02's selection, where the request names nothing.
        let next = self.partitions.get(&id).map(|p| p.next).unwrap_or_default();
        let source = source.or(next.source);
        let keylock = keylock.or(next.keylock);
        let result = self
            .resolve(id, &profile, source)
            .and_then(|(profile, current)| {
                let overrides = super::life::Overrides {
                    source,
                    keylock,
                    media: None,
                    ipl_type: next.ipl_type,
                };
                self.launch(id, &profile, current.clone(), overrides)
                    .map(|()| current)
            });
        match &result {
            Ok(_) => {
                if let Some(p) = self.partitions.get_mut(&id) {
                    p.next = PanelArgs::default();
                }
            }
            Err(errors) => {
                // A partition that is activated already is not one whose
                // activation failed.
                let active = self.partitions.get(&id).is_some_and(Partition::active);
                if !active {
                    self.not_activated(id, errors);
                }
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
            // Made once and kept.
            mac_prefix: self.host.settings.mac_prefix.clone(),
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
            .filter(|p| p.active())
            .map(|p| (p.run.as_ref().map_or(0, |r| r.sequence), p.definition.id))
            .collect();
        // The last one activated is the first to end.
        active.sort_unstable_by(|a, b| b.cmp(a));
        let queue: Vec<u32> = active.into_iter().map(|(_, id)| id).collect();
        let count = u32::try_from(queue.len()).unwrap_or(u32::MAX);
        let first = queue.first().copied();
        self.ending = Some(queue);
        if let Some(first) = first {
            self.end_for_host(first);
        }
        count
    }

    fn subscribe(&mut self) {}

    fn load_media(&mut self, id: u32, slot: u32, name: String) -> Result<(), Refused> {
        let (partition, kind) = self.media_slot(id, slot)?;
        if !model::validate::valid_store_name(&name) {
            return Err(one(qvsmsg::MEDIA_NAME_INVALID.with(&[&name])));
        }
        let want = Want {
            kind,
            pool: String::new(),
            name: name.clone(),
            slot,
        };
        let given = self.system.hold(id, std::slice::from_ref(&want))?;
        let (cap, protected) = given
            .first()
            .map_or((0, false), |g| (g.capacity_mib, g.protected));
        let line = format!("load {slot} {name} {cap} {protected}");
        match self.system.control(id, &line, Some(&want)) {
            Ok(answer) if answer.starts_with("ok") => {
                // What was there before is given back.
                let old = self.set_medium(id, slot, Some(name));
                if let Some(old) = old {
                    self.system.give_back(id, &[Want { name: old, ..want }]);
                }
                Ok(())
            }
            other => {
                self.system.give_back(id, &[want]);
                let why = other.unwrap_or_else(|e| e);
                Err(one(qvsmsg::MEDIA_REFUSED.with(&[
                    &partition,
                    &slot.to_string(),
                    why.trim_start_matches("refused ").trim(),
                ])))
            }
        }
    }

    fn unload_media(&mut self, id: u32, slot: u32, forced: bool) -> Result<String, Refused> {
        let (partition, kind) = self.media_slot(id, slot)?;
        let answer = self
            .system
            .control(id, &format!("unload {slot} {forced}"), None);
        match answer {
            Ok(a) if a.starts_with("ok ") => {
                let name = a.trim_start_matches("ok ").trim().to_string();
                self.set_medium(id, slot, None);
                self.system.give_back(
                    id,
                    &[Want {
                        kind,
                        pool: String::new(),
                        name: name.clone(),
                        slot,
                    }],
                );
                Ok(name)
            }
            Ok(a) if a.contains("prevented") => Err(one(
                qvsmsg::MEDIA_PREVENTED.with(&[&partition, &slot.to_string()])
            )),
            other => {
                let why = other.unwrap_or_else(|e| e);
                Err(one(qvsmsg::MEDIA_REFUSED.with(&[
                    &partition,
                    &slot.to_string(),
                    why.trim_start_matches("refused ").trim(),
                ])))
            }
        }
    }

    fn media(&mut self, id: u32) -> Result<Vec<MediaSlot>, Refused> {
        self.media_slot(id, 0).or_else(|e| {
            // Slot 0 is never a device: only whether it runs is asked.
            if e.iter().any(|m| m.is(&qvsmsg::MEDIA_SLOT_MISSING)) {
                Ok((String::new(), poold::proto::Kind::Image))
            } else {
                Err(e)
            }
        })?;
        let answer = self.system.control(id, "media", None).unwrap_or_default();
        Ok(answer
            .lines()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                let slot = w.next()?.parse().ok()?;
                let kind = w.next()?.to_string();
                let name = w.next()?;
                let prevented = w.next()? == "true";
                Some(MediaSlot {
                    slot,
                    kind,
                    name: if name == "-" {
                        String::new()
                    } else {
                        name.to_string()
                    },
                    prevented,
                })
            })
            .collect())
    }

    fn adapters(&mut self, id: u32) -> Vec<vswitchd::Adapter> {
        let mut out = Vec::new();
        for p in self.partitions.values() {
            if id != 0 && p.definition.id != id {
                continue;
            }
            for profile in p.profiles.values() {
                for a in &profile.ethernet {
                    out.push(vswitchd::Adapter {
                        partition: p.definition.id,
                        name: p.definition.name.clone(),
                        profile: profile.name.clone(),
                        slot: a.slot,
                        switch: a.switch.clone().unwrap_or_default(),
                        vlan: a.port_vlan().unwrap_or(0),
                        tagged: a.tagged_vlans(),
                        mac: a.mac.clone().unwrap_or_default(),
                        required: a.required,
                    });
                }
            }
        }
        out
    }

    fn shut_down_as(
        &mut self,
        id: u32,
        kind: Shutdown,
        delay: u32,
        restart: bool,
    ) -> Result<(), Refused> {
        // The socket's service waits for the system's answer; called here
        // the request is only sent.
        match self.begin_shut_down(id, kind, delay, restart) {
            super::Answer::Now(result) => result.map(|_| ()),
            super::Answer::Later(_) => Ok(()),
        }
    }

    fn panel(&mut self, id: u32, function: u8, args: PanelArgs) -> Result<String, Refused> {
        match self.begin_panel(id, function, args) {
            super::Answer::Now(result) => result,
            super::Answer::Later(_) => Ok(String::new()),
        }
    }

    fn take_dump(&mut self, id: u32) -> Result<String, Refused> {
        self.begin_dump(id, "request", false)
    }

    fn dumps(&mut self, id: u32) -> Result<Vec<DumpInfo>, Refused> {
        self.partition(id)?;
        Ok(self.system.dumps(id))
    }

    fn delete_dump(&mut self, id: u32, name: String) -> Result<(), Refused> {
        let partition = self.partition(id)?.definition.name.clone();
        if !super::dumps::valid_name(&name) || !self.system.remove_dump(id, &name) {
            return Err(one(qvsmsg::DUMP_MISSING.with(&[&partition, &name])));
        }
        Ok(())
    }

    fn read_dump(&mut self, id: u32, name: String) -> Result<u64, Refused> {
        let partition = self.partition(id)?.definition.name.clone();
        self.system
            .dumps(id)
            .into_iter()
            .find(|d| d.name == name)
            .map(|d| d.bytes)
            .ok_or_else(|| one(qvsmsg::DUMP_MISSING.with(&[&partition, &name])))
    }

    fn console(&mut self, id: u32, _take_over: bool) -> Result<bool, Refused> {
        // Attaching is the socket's service's: it holds the connection.
        self.console_mode(id)
    }

    fn screen(&mut self, id: u32) -> Result<ScreenCapture, Refused> {
        let serial = self.console_mode(id)?;
        let p = self.partition(id)?;
        let frame = p.console.frame();
        if !serial && frame.is_none() {
            return Err(one(qvsmsg::CONSOLE_EMPTY.with(&[&p.definition.name])));
        }
        Ok(ScreenCapture {
            serial,
            text: if serial {
                String::new()
            } else {
                p.console.text()
            },
            frame: if serial { None } else { frame },
        })
    }

    fn named(&mut self, pool: String, name: String) -> Vec<String> {
        let wanted = format!("{pool}/{name}");
        let mut out = Vec::new();
        for partition in self.partitions.values() {
            for profile in partition.profiles.values() {
                for (slot, unit) in profile.units() {
                    if unit.backing == wanted {
                        out.push(format!(
                            "{} profile {} slot {slot}",
                            partition.definition.name, profile.name
                        ));
                    }
                }
            }
        }
        out
    }
}

impl<S: System> Manager<S> {
    /// Shut `id` down at once: a held partition too, and one whose
    /// system is shutting it down.
    pub(crate) fn end_one(&mut self, id: u32, again: bool) -> Result<(), Refused> {
        let partition = self.partition(id)?;
        let name = &partition.definition.name;
        match &partition.run {
            None => Err(one(
                qvsmsg::PARTITION_NOT_ACTIVE.with(&[name, partition.state.word()])
            )),
            Some(run) if run.asked.is_some() => Err(one(qvsmsg::PARTITION_ENDING.with(&[name]))),
            Some(_) => {
                self.begin_end(id, again);
                Ok(())
            }
        }
    }
}
