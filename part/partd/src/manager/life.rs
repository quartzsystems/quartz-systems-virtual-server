// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's life: activation, what its VMM reports, how the VMM
//! ends, and what follows each kind of end.
//!
//! | The VMM ends because | The partition is then |
//! |---|---|
//! | the guest powered off | Not activated |
//! | the guest asked for a restart | activated again at once, with the same resources and the side it asked for |
//! | the partition manager shut it down | Not activated; activated again when that was a restart |
//! | it could not IPL the partition | Not activated, with the message |
//! | it failed, the guest faulted, or it was killed | Error, with the reference code and how it ended; not started again |

use model::validate::{Backing, backing};
use model::{EthernetAdapter, IplSource, IplType, Keylock, Profile};
use poold::proto::{Kind, Want};
use qsvmm::config::Net;
use qsvmm::end::End;
use qvsmsg::Issued;
use resource::cgroup;
use resource::{Current, Held};
use vswitch::port::PortSpec;

use super::{Ended, MAX_LINE, Manager, Record, Run, System, config};
use config::Stores;

/// The media a profile loads at activation: each optical and tape
/// device's slot and its medium.
pub(crate) fn profile_media(profile: &Profile) -> Vec<(u32, Option<String>)> {
    profile
        .optical
        .iter()
        .chain(&profile.tape)
        .map(|d| (d.slot, d.media.clone()))
        .collect()
}

/// What a medium in `slot` is to the pool service: an image for an
/// optical device, a volume for a tape device.
pub(crate) fn media_kind(profile: &Profile, slot: u32) -> Option<Kind> {
    if profile.optical.iter().any(|o| o.slot == slot) {
        Some(Kind::Image)
    } else if profile.tape.iter().any(|t| t.slot == slot) {
        Some(Kind::Volume)
    } else {
        None
    }
}

/// What an activation sets in place of the profile's and the
/// definition's: none of it, unless said.
#[derive(Clone, Debug, Default)]
pub(crate) struct Overrides {
    /// The IPL source.
    pub(crate) source: Option<IplSource>,
    /// The keylock position.
    pub(crate) keylock: Option<Keylock>,
    /// The media each device has, in place of the profile's.
    pub(crate) media: Option<Vec<(u32, Option<String>)>>,
    /// The IPL type.
    pub(crate) ipl_type: Option<IplType>,
}

/// What the pool service is asked to hold for one key of a launch.
enum Key {
    Unit(u32, u32),
    Optical(u32, String),
    Tape(u32, String),
}
use crate::proto::{Event, State};

/// A line of a VMM's report as a reference code and its text, when it
/// is one of the table's.
pub fn code_of(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix(refcode::WORD)?.strip_prefix(' ')?;
    let (code, text) = rest.split_once(' ')?;
    refcode::find(code).map(|_| (code, text))
}

impl<S: System> Manager<S> {
    /// Resolve and check an activation of `id` with the profile
    /// `profile_name`; what it would be given.
    pub(crate) fn resolve(
        &self,
        id: u32,
        profile_name: &str,
        source: Option<IplSource>,
    ) -> Result<(String, Current), Vec<Issued>> {
        let partition = self
            .partitions
            .get(&id)
            .ok_or_else(|| vec![qvsmsg::PARTITION_MISSING.with(&[&id.to_string()])])?;
        let name = &partition.definition.name;
        if partition.active() {
            return Err(vec![
                qvsmsg::PARTITION_ACTIVE.with(&[name, partition.state.word()]),
            ]);
        }
        if self.ending.is_some() {
            return Err(vec![qvsmsg::HOST_STOPPING.with(&[name])]);
        }
        let profile_name = if profile_name.is_empty() {
            partition.definition.default_profile.as_str()
        } else {
            profile_name
        };
        if profile_name.is_empty() {
            return Err(vec![qvsmsg::NO_DEFAULT_PROFILE.with(&[name])]);
        }
        let profile = partition
            .profiles
            .get(profile_name)
            .ok_or_else(|| vec![qvsmsg::PROFILE_MISSING.with(&[name, profile_name])])?;
        let errors = model::validate::profile(profile, self.limits());
        if !errors.is_empty() {
            return Err(errors);
        }
        if let Some(refused) = model::validate::not_yet(profile) {
            return Err(vec![refused]);
        }
        if source == Some(IplSource::D) && profile.optical.is_empty() {
            return Err(vec![qvsmsg::NO_OPTICAL.with(&[])]);
        }
        // A host path is a test image's; one that is not there fails
        // the activation here, by name, not a VMM a moment later. A
        // virtual disk is checked by the pool service as it is held.
        let mut missing = Vec::new();
        for (slot, unit) in profile.units() {
            let (slot, lun) = (slot.to_string(), unit.lun.to_string());
            if let Some(Backing::Path(path)) = backing(&unit.backing) {
                if !self.system.host_paths() {
                    missing.push(qvsmsg::BACKING_NOT_VDISK.with(&[&slot, &lun, path]));
                } else if !self.system.exists(path) {
                    missing.push(qvsmsg::BACKING_MISSING.with(&[name, &slot, &lun, path]));
                }
            }
        }
        if !missing.is_empty() {
            return Err(missing);
        }
        let current = self
            .host
            .resolve(&self.held(), name, profile)
            .map_err(|e| vec![e])?;
        Ok((profile_name.to_string(), current))
    }

    /// Have the pool service hold the virtual disks and media of an
    /// activation of `id` with `profile`, the media as `media` says.
    /// What the VMM is to name each by.
    fn hold_stores(
        &mut self,
        id: u32,
        profile: &Profile,
        media: &[(u32, Option<String>)],
    ) -> Result<Stores, Vec<Issued>> {
        let mut wants = Vec::new();
        let mut keys = Vec::new();
        for (slot, unit) in profile.units() {
            if let Some(Backing::Vdisk { pool, name }) = backing(&unit.backing) {
                wants.push(Want {
                    kind: Kind::Vdisk,
                    pool: pool.to_string(),
                    name: name.to_string(),
                    slot,
                });
                keys.push(Key::Unit(slot, unit.lun));
            }
        }
        for (slot, medium) in media {
            let (Some(name), Some(kind)) = (medium, media_kind(profile, *slot)) else {
                continue;
            };
            wants.push(Want {
                kind,
                pool: String::new(),
                name: name.clone(),
                slot: *slot,
            });
            keys.push(match kind {
                Kind::Image => Key::Optical(*slot, name.clone()),
                _ => Key::Tape(*slot, name.clone()),
            });
        }
        let mut stores = Stores::default();
        if wants.is_empty() {
            return Ok(stores);
        }
        let given = self.system.hold(id, &wants)?;
        stores.files = given.len();
        for (i, (key, g)) in keys.into_iter().zip(&given).enumerate() {
            // The files are the VMM's from descriptor 3 on, in order.
            let path = format!("/proc/self/fd/{}", 3 + i);
            match key {
                Key::Unit(slot, lun) => {
                    stores.units.insert((slot, lun), path);
                }
                Key::Optical(slot, name) => {
                    stores.optical.insert(slot, (path, name));
                }
                Key::Tape(slot, name) => {
                    stores
                        .tape
                        .insert(slot, (path, name, g.capacity_mib, g.protected));
                }
            }
        }
        Ok(stores)
    }

    /// Open the ports of `profile`'s Ethernet adapters for `id`, named
    /// `name`, in slot order: each a `[[net]]` of `stores`, its tap the
    /// VMM's descriptor after the pool service's files. A required
    /// adapter that gets no port fails the activation by name; one that
    /// is not comes up without link. Each adapter's slot and whether its
    /// port was opened.
    fn open_ports(
        &mut self,
        id: u32,
        name: &str,
        profile: &Profile,
        stores: &mut Stores,
    ) -> Result<Vec<(u32, bool)>, Vec<Issued>> {
        let mut adapters: Vec<&EthernetAdapter> = profile.ethernet.iter().collect();
        adapters.sort_by_key(|a| a.slot);
        let prefix = self.prefix();
        let mut ports = Vec::new();
        let mut taps = 0;
        for a in adapters {
            let mac = a
                .mac
                .as_deref()
                .and_then(vswitch::mac::parse)
                .unwrap_or_else(|| vswitch::mac::made(prefix, id, a.slot));
            let spec = PortSpec {
                partition: id,
                slot: a.slot,
                switch: a.switch.clone().unwrap_or_default(),
                vlan: a.port_vlan().unwrap_or(0),
                tagged: a.tagged_vlans(),
                mac,
            };
            match self.system.open_port(id, &spec) {
                Ok(mtu) => {
                    let fd = 3 + stores.files + taps;
                    taps += 1;
                    stores.net.push(Net {
                        slot: a.slot,
                        mac: vswitch::mac::text(&mac),
                        tap: Some(format!("/proc/self/fd/{fd}").into()),
                        mtu,
                    });
                    ports.push((a.slot, true));
                }
                Err(refused) => {
                    let why = refused
                        .first()
                        .map(|m| m.text.trim_end_matches('.').to_string())
                        .unwrap_or_default();
                    let slot = a.slot.to_string();
                    if a.required {
                        self.system.close_ports(id);
                        let mut all = vec![qvsmsg::PORT_NOT_GIVEN.with(&[name, &slot, &why])];
                        all.extend(refused);
                        return Err(all);
                    }
                    self.system.log(&format!(
                        "{name}: the adapter in slot {slot} comes up without link: {why}"
                    ));
                    stores.net.push(Net {
                        slot: a.slot,
                        mac: vswitch::mac::text(&mac),
                        tap: None,
                        mtu: vswitch::switch::DEFAULT_MTU,
                    });
                    ports.push((a.slot, false));
                }
            }
        }
        Ok(ports)
    }

    /// The control groups set for `id` holding `current`, and its VMM
    /// started in `group` with `text`: its process.
    fn start_vmm(
        &mut self,
        id: u32,
        group: &str,
        current: &Current,
        text: &str,
    ) -> Result<u32, String> {
        // The pools as they are with this partition holding its part:
        // a dedicated partition's cores leave the default pool first.
        let mut held = self.held();
        held.push(Held {
            partition: id,
            current: current.clone(),
        });
        self.system
            .apply(&cgroup::base(&self.host, &held))
            .and_then(|()| {
                self.system
                    .apply(&cgroup::partition(&self.host, id, current))
            })
            .and_then(|()| self.system.launch(id, group, text))
    }

    /// The service channel's sockets into `stores`: the VMM's
    /// descriptors after the taps. What was held is given back when they
    /// cannot be made.
    fn channel_stores(
        &mut self,
        id: u32,
        name: &str,
        stores: &mut Stores,
    ) -> Result<(), Vec<Issued>> {
        let taps = stores.net.iter().filter(|n| n.tap.is_some()).count();
        match self.system.channel_sockets(id, 3 + stores.files + taps) {
            Ok(ports) => {
                stores.ports = ports;
                Ok(())
            }
            Err(e) => {
                self.system.give_back(id, &[]);
                self.system.close_ports(id);
                Err(vec![qvsmsg::VMM_NOT_STARTED.with(&[name, &e])])
            }
        }
    }

    /// Start the VMM of `id` holding `current`, with its media as
    /// `media` says, or as the profile does. The partition is then
    /// Starting.
    pub(crate) fn launch(
        &mut self,
        id: u32,
        profile_name: &str,
        current: Current,
        overrides: Overrides,
    ) -> Result<(), Vec<Issued>> {
        let Overrides {
            source,
            keylock,
            media,
            ipl_type,
        } = overrides;
        let Some(partition) = self.partitions.get(&id) else {
            return Err(vec![qvsmsg::PARTITION_MISSING.with(&[&id.to_string()])]);
        };
        let name = partition.definition.name.clone();
        let Some(profile) = partition.profiles.get(profile_name).cloned() else {
            return Err(vec![qvsmsg::PROFILE_MISSING.with(&[&name, profile_name])]);
        };
        let definition = partition.definition.clone();
        let media = media.unwrap_or_else(|| profile_media(&profile));
        let mut stores = self.hold_stores(id, &profile, &media)?;
        let ports = match self.open_ports(id, &name, &profile, &mut stores) {
            Ok(ports) => ports,
            Err(refused) => {
                self.system.give_back(id, &[]);
                return Err(refused);
            }
        };
        self.channel_stores(id, &name, &mut stores)?;
        stores.ipl_type = ipl_type;
        let vcpus = self.host.vcpus(&current.processors);
        let written = config::build(
            &definition,
            &profile,
            &current,
            &vcpus,
            source.unwrap_or(profile.boot.source),
            keylock.unwrap_or(definition.keylock),
            &stores,
        );
        let text = qsvmm::config::render(&written);
        let group = cgroup::partition_group(id, &current.processors);
        let pid = match self.start_vmm(id, &group, &current, &text) {
            Ok(pid) => pid,
            Err(e) => {
                self.system.remove_group(&group);
                self.system.give_back(id, &[]);
                self.system.close_ports(id);
                let _ = self.apply_base();
                return Err(vec![qvsmsg::VMM_NOT_STARTED.with(&[&name, &e])]);
            }
        };
        let sequence = self.sequence;
        self.sequence += 1;
        let record = Record {
            id,
            profile: profile_name.to_string(),
            current: current.clone(),
            pid,
            sequence,
            state: State::Starting,
        };
        self.system.record(id, Some(&record));
        let time = self.system.now();
        if let Some(partition) = self.partitions.get_mut(&id) {
            partition.run = Some(Run {
                current: current.clone(),
                profile: profile_name.to_string(),
                pid,
                group,
                sequence,
                keylock,
                ipl_type,
                again: false,
                asked: None,
                plan: None,
                media,
                lost: None,
                ports,
                kept: false,
                uplinks_down: Vec::new(),
            });
            partition.last_profile = profile_name.to_string();
            partition.message = None;
            partition.last_end.clear();
            partition.service = super::service::Service::fresh();
            partition.console = console_port::Console::new();
        }
        self.set_state(id, State::Starting);
        self.publish(&Event::Resources {
            id,
            name,
            current: Some(current),
            time,
        });
        Ok(())
    }

    /// An activation that failed: the partition stays as it was, with
    /// the message, and the host is told.
    pub(crate) fn not_activated(&mut self, id: u32, errors: &[Issued]) {
        let Some(partition) = self.partitions.get_mut(&id) else {
            return;
        };
        let Some(first) = errors.first() else {
            return;
        };
        partition.message = Some(first.clone());
        let detail = format!("{}, {first}", partition.definition.name);
        let code = &refcode::PARTITION_NOT_ACTIVATED;
        self.code(id, code.code, &code.text_with(&detail), true);
    }

    /// A line the VMM of `id` reported. A reference code goes into the
    /// history; the code that says the guest's kernel was entered starts
    /// the wait for its service channel; the one that says it is held
    /// after a fault holds it in Error. With `quiet`, the line is one
    /// read again by a manager that found the partition running: it is
    /// in the history already, and nothing is posted for it again.
    pub fn report(&mut self, id: u32, line: &str, quiet: bool) {
        // The line is the VMM's, which a guest may have got at: it is
        // bounded, and only a code of the table is taken from it.
        let line: String = line.chars().take(MAX_LINE).collect();
        if let Some(rest) = line.strip_prefix("MEDIA ") {
            self.media_unloaded(id, rest);
            return;
        }
        let Some((code, text)) = code_of(&line) else {
            if !quiet {
                let name = self
                    .partitions
                    .get(&id)
                    .map(|p| p.definition.name.clone())
                    .unwrap_or_default();
                self.system.log(&format!("{name}: {line}"));
            }
            return;
        };
        let (code, text) = (code.to_string(), text.to_string());
        if !quiet {
            self.code(id, &code, &text, true);
        }
        let now = self.system.now();
        if code == refcode::PARTITION_STARTED.code
            && let Some(partition) = self.partitions.get_mut(&id)
        {
            partition.service.kernel_at = Some(now);
        }
        if code == refcode::PARTITION_FAULT_HELD.code {
            self.kept(id);
        }
    }

    /// Give back what the partition `id` held.
    fn release(&mut self, id: u32) -> Option<Run> {
        let time = self.system.now();
        let partition = self.partitions.get_mut(&id)?;
        let run = partition.run.take()?;
        let name = partition.definition.name.clone();
        if let Some((_, viewer)) = partition.viewer.take() {
            let _ = viewer.try_send(crate::proto::ConsoleOut::Ended {
                why: "the partition ended".into(),
            });
        }
        let waiting: Vec<u64> = partition.service.asks.iter().map(|(t, ..)| *t).collect();
        partition.service.channel = None;
        partition.service.facts.up = false;
        partition.service.facts.responding = false;
        partition.service.facts.kept = false;
        for tag in waiting {
            if let Some(at) = partition.service.asks.iter().position(|(t, ..)| *t == tag) {
                let (_, _, to) = partition.service.asks.remove(at);
                if let Some(to) = to {
                    let _ = to.try_send(Err(vec![qvsmsg::PARTITION_ENDING.with(&[&name])]));
                }
            }
        }
        self.system.remove_group(&run.group);
        self.system.record(id, None);
        self.system.give_back(id, &[]);
        self.system.close_ports(id);
        if let Err(e) = self.apply_base() {
            self.system.log(&format!("resource controls: {e}"));
        }
        self.publish(&Event::Resources {
            id,
            name,
            current: None,
            time,
        });
        Some(run)
    }

    /// The VMM of `id` has ended.
    pub fn ended(&mut self, id: u32, how: Ended) {
        let Some(partition) = self.partitions.get(&id) else {
            return;
        };
        let name = partition.definition.name.clone();
        // Asked by the host to end at once; a shut down the system
        // accepted ends by the system's own doing.
        let asked = partition.run.as_ref().is_some_and(|r| r.asked.is_some());
        // The last attention code before the end says why.
        let before = partition
            .history
            .iter()
            .rev()
            .take_while(|p| p.code != refcode::PARTITION_ACTIVATING.code)
            .find(|p| p.code.starts_with('A'))
            .map(|p| p.code.clone());
        let Some(run) = self.release(id) else {
            return;
        };
        let end = match how {
            Ended::Exit(code) => End::from_exit(Some(code)),
            Ended::Signal(_) | Ended::Unknown => End::Killed,
        };
        let code = end.code();
        self.code(
            id,
            code.code,
            &code.text_with(&format!("{name}, {}", how.how())),
            true,
        );
        if let Some(partition) = self.partitions.get_mut(&id) {
            partition.last_end = how.how();
        }

        let stopping = self.ending.is_some();
        let again = if let Some(pool) = &run.lost {
            // Ended for a pool that failed under it.
            let message = qvsmsg::POOL_UNAVAILABLE.with(&[&name, pool, "Failed"]);
            if let Some(partition) = self.partitions.get_mut(&id) {
                partition.message = Some(message);
            }
            self.set_state(id, State::Error);
            false
        } else if asked {
            // The partition manager shut it down, however it went.
            self.set_state(id, State::NotActivated);
            run.again && !stopping
        } else {
            match end {
                End::PoweredOff | End::Ended => {
                    self.set_state(id, State::NotActivated);
                    // A shut down with restart asked of the system that
                    // powered off: activated again all the same.
                    run.plan.is_some_and(|p| p.restart) && !stopping
                }
                End::Restart => !stopping,
                End::NotIpled => {
                    let reference = before.unwrap_or_else(|| code.code.to_string());
                    let message = qvsmsg::DID_NOT_IPL.with(&[&name, &reference]);
                    if let Some(partition) = self.partitions.get_mut(&id) {
                        partition.message = Some(message);
                    }
                    self.set_state(id, State::NotActivated);
                    false
                }
                End::GuestFault | End::VmmError | End::Killed => {
                    let message = qvsmsg::VMM_FAILED.with(&[&name, code.code, &how.how()]);
                    if let Some(partition) = self.partitions.get_mut(&id) {
                        partition.message = Some(message);
                    }
                    self.set_state(id, State::Error);
                    false
                }
            }
        };
        if again {
            self.again(id, &run, asked && run.lost.is_none());
        } else if end == End::Restart && !asked {
            // A restart that is not to be: the host is stopping.
            self.set_state(id, State::NotActivated);
        }
        self.end_next(id);
    }

    /// Activate `id` again after an end that asks for it. The guest's
    /// own restart keeps what the partition held and IPLs from the
    /// side it asked for; a restart the partition manager was asked
    /// for is a new activation with the profile it had.
    fn again(&mut self, id: u32, run: &Run, asked: bool) {
        let result = if asked {
            // It is Not activated now: resolved anew.
            self.resolve(id, &run.profile, None)
                .and_then(|(profile, current)| {
                    // This activation's keylock and IPL type are kept.
                    let overrides = Overrides {
                        keylock: run.keylock,
                        ipl_type: run.ipl_type,
                        ..Overrides::default()
                    };
                    self.launch(id, &profile, current, overrides)
                })
        } else {
            // Still Starting or Running: the state goes back to
            // Starting through the launch.
            if let Some(partition) = self.partitions.get_mut(&id) {
                partition.state = State::NotActivated;
            }
            // What the guest had loaded, it has again.
            self.launch(
                id,
                &run.profile,
                run.current.clone(),
                Overrides {
                    source: Some(IplSource::Recorded),
                    keylock: run.keylock,
                    media: Some(run.media.clone()),
                    ipl_type: run.ipl_type,
                },
            )
        };
        if let Err(errors) = result {
            self.not_activated(id, &errors);
            self.set_state(id, State::NotActivated);
        }
    }

    /// Ask the VMM of `id` to end; the partition is Shutting down
    /// until it has.
    pub(crate) fn begin_end(&mut self, id: u32, again: bool) {
        let now = self.system.now();
        let Some(run) = self.partitions.get_mut(&id).and_then(|p| p.run.as_mut()) else {
            return;
        };
        run.again = again;
        run.asked = Some(now);
        let pid = run.pid;
        self.system.end(pid);
        self.set_state(id, State::ShuttingDown);
    }

    /// The host's shutdown goes on to the next partition when `id`
    /// was the one being ended.
    fn end_next(&mut self, id: u32) {
        let Some(queue) = &mut self.ending else {
            return;
        };
        if queue.first() != Some(&id) {
            queue.retain(|other| *other != id);
            return;
        }
        queue.remove(0);
        while let Some(next) = self.ending.as_ref().and_then(|q| q.first().copied()) {
            let active = self
                .partitions
                .get(&next)
                .is_some_and(super::Partition::active);
            if active {
                self.end_for_host(next);
                return;
            }
            if let Some(queue) = &mut self.ending {
                queue.remove(0);
            }
        }
    }

    /// The VMM of `id` says the guest took the medium out of a device:
    /// `<slot> unloaded <name>`. It is given back.
    fn media_unloaded(&mut self, id: u32, said: &str) {
        let mut words = said.split_whitespace();
        let (Some(slot), Some("unloaded")) =
            (words.next().and_then(|s| s.parse().ok()), words.next())
        else {
            return;
        };
        let Some(partition) = self.partitions.get_mut(&id) else {
            return;
        };
        let profile = partition.profiles.get(&partition.last_profile).cloned();
        let Some(run) = partition.run.as_mut() else {
            return;
        };
        let mut gone = None;
        for (s, medium) in &mut run.media {
            if *s == slot {
                gone = medium.take();
            }
        }
        let kind = profile.as_ref().and_then(|p| media_kind(p, slot));
        if let (Some(name), Some(kind)) = (gone, kind) {
            let want = Want {
                kind,
                pool: String::new(),
                name,
                slot,
            };
            self.system.give_back(id, &[want]);
        }
    }

    /// A storage pool failed: every active partition with a unit in it
    /// is ended, and is in Error when it has.
    pub fn pool_failed(&mut self, pool: &str) {
        let prefix = format!("{pool}/");
        let mut kill = Vec::new();
        for partition in self.partitions.values_mut() {
            let uses = partition
                .profiles
                .get(&partition.last_profile)
                .is_some_and(|p| {
                    p.units()
                        .iter()
                        .any(|(_, u)| u.backing.starts_with(&prefix))
                });
            if let Some(run) = partition.run.as_mut()
                && uses
                && run.lost.is_none()
            {
                run.lost = Some(pool.to_string());
                kill.push(run.group.clone());
            }
        }
        for group in kill {
            self.system.kill(&group);
        }
    }

    /// What time does: a VMM that was asked to end and has not is
    /// killed; the service channels' own timers. Called every second or
    /// so.
    pub fn tick(&mut self) {
        self.service_tick();
        let now = self.system.now();
        let overdue: Vec<String> = self
            .partitions
            .values()
            .filter_map(|p| p.run.as_ref())
            .filter(|r| {
                r.asked
                    .is_some_and(|at| now.saturating_sub(at) >= super::END_GRACE_SECS)
            })
            .map(|r| r.group.clone())
            .collect();
        for group in overdue {
            self.system.kill(&group);
        }
    }

    /// Take up a partition that ran while no partition manager did:
    /// it holds what its record says, and the lines of its report so
    /// far say where it is.
    pub fn adopt(&mut self, record: &Record, report: &[String]) {
        let id = record.id;
        let Some(partition) = self.partitions.get_mut(&id) else {
            self.system.log(&format!(
                "a VMM runs for partition {id}, which is not defined"
            ));
            return;
        };
        let name = partition.definition.name.clone();
        partition.run = Some(Run {
            current: record.current.clone(),
            profile: record.profile.clone(),
            pid: record.pid,
            group: cgroup::partition_group(id, &record.current.processors),
            sequence: record.sequence,
            keylock: None,
            ipl_type: None,
            again: false,
            asked: None,
            plan: None,
            kept: false,
            uplinks_down: Vec::new(),
            media: partition
                .profiles
                .get(&record.profile)
                .map(profile_media)
                .unwrap_or_default(),
            lost: None,
            // Its ports stay with its VMM's taps.
            ports: partition
                .profiles
                .get(&record.profile)
                .map(|p| p.ethernet.iter().map(|a| (a.slot, true)).collect())
                .unwrap_or_default(),
        });
        partition.last_profile.clone_from(&record.profile);
        // The state it was in, as the last manager recorded it.
        partition.state = if record.state.active() {
            record.state
        } else {
            State::Starting
        };
        partition.service = super::service::Service::fresh();
        self.sequence = self.sequence.max(record.sequence + 1);
        for line in report {
            self.report(id, line, true);
        }
        // One kept before this manager started is kept still.
        if self
            .system
            .control(id, "kept", None)
            .is_ok_and(|a| a == "ok true")
        {
            self.kept(id);
        }
        let code = &refcode::PARTITION_ADOPTED;
        self.code(id, code.code, &code.text_with(&name), true);
    }
}
