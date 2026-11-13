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

use model::{IplSource, Keylock};
use qsvmm::end::End;
use qvsmsg::Issued;
use resource::cgroup;
use resource::{Current, Held};

use super::{Ended, MAX_LINE, Manager, Record, Run, System, config};
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
        if partition.state.active() {
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
        if source == Some(IplSource::D) {
            return Err(vec![qvsmsg::NOT_YET.with(&["IPL source D", "5"])]);
        }
        // A unit that is not there fails the activation here, by name,
        // not a VMM a moment later.
        let missing: Vec<Issued> = profile
            .units()
            .into_iter()
            .filter(|(_, unit)| !self.system.exists(&unit.backing))
            .map(|(slot, unit)| {
                qvsmsg::BACKING_MISSING.with(&[
                    name,
                    &slot.to_string(),
                    &unit.lun.to_string(),
                    &unit.backing,
                ])
            })
            .collect();
        if !missing.is_empty() {
            return Err(missing);
        }
        let current = self
            .host
            .resolve(&self.held(), name, profile)
            .map_err(|e| vec![e])?;
        Ok((profile_name.to_string(), current))
    }

    /// Start the VMM of `id` holding `current`. The partition is then
    /// Starting.
    pub(crate) fn launch(
        &mut self,
        id: u32,
        profile_name: &str,
        current: Current,
        source: Option<IplSource>,
        keylock: Option<Keylock>,
    ) -> Result<(), Issued> {
        let Some(partition) = self.partitions.get(&id) else {
            return Err(qvsmsg::PARTITION_MISSING.with(&[&id.to_string()]));
        };
        let name = partition.definition.name.clone();
        let Some(profile) = partition.profiles.get(profile_name) else {
            return Err(qvsmsg::PROFILE_MISSING.with(&[&name, profile_name]));
        };
        let vcpus = self.host.vcpus(&current.processors);
        let written = config::build(
            &partition.definition,
            profile,
            &current,
            &vcpus,
            source.unwrap_or(profile.boot.source),
            keylock.unwrap_or(partition.definition.keylock),
        );
        let text = qsvmm::config::render(&written);
        let group = cgroup::partition_group(id, &current.processors);

        // The pools as they are with this partition holding its part:
        // a dedicated partition's cores leave the default pool first.
        let mut held = self.held();
        held.push(Held {
            partition: id,
            current: current.clone(),
        });
        let started = self
            .system
            .apply(&cgroup::base(&self.host, &held))
            .and_then(|()| {
                self.system
                    .apply(&cgroup::partition(&self.host, id, &current))
            })
            .and_then(|()| self.system.launch(id, &group, &text));
        let pid = match started {
            Ok(pid) => pid,
            Err(e) => {
                self.system.remove_group(&group);
                let _ = self.apply_base();
                return Err(qvsmsg::VMM_NOT_STARTED.with(&[&name, &e]));
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
                again: false,
                asked: None,
            });
            partition.last_profile = profile_name.to_string();
            partition.message = None;
            partition.last_end.clear();
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

    /// A line the VMM of `id` reported. A reference code becomes the
    /// partition's current one; the code that says the guest's kernel
    /// was entered makes the partition Running. With `quiet`, the line
    /// is one read again by a manager that found the partition
    /// running: nothing is posted for it a second time.
    pub fn report(&mut self, id: u32, line: &str, quiet: bool) {
        // The line is the VMM's, which a guest may have got at: it is
        // bounded, and only a code of the table is taken from it.
        let line: String = line.chars().take(MAX_LINE).collect();
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
        self.code(id, &code, &text, !quiet);
        let starting = self
            .partitions
            .get(&id)
            .is_some_and(|p| p.state == State::Starting);
        if code == refcode::PARTITION_STARTED.code && starting {
            if quiet {
                if let Some(partition) = self.partitions.get_mut(&id) {
                    partition.state = State::Running;
                }
            } else {
                self.set_state(id, State::Running);
            }
        }
    }

    /// Give back what the partition `id` held.
    fn release(&mut self, id: u32) -> Option<Run> {
        let time = self.system.now();
        let partition = self.partitions.get_mut(&id)?;
        let run = partition.run.take()?;
        let name = partition.definition.name.clone();
        self.system.remove_group(&run.group);
        self.system.record(id, None);
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
        let asked = partition.state == State::ShuttingDown;
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
        let again = if asked {
            // The partition manager shut it down, however it went.
            self.set_state(id, State::NotActivated);
            run.again && !stopping
        } else {
            match end {
                End::PoweredOff | End::Ended => {
                    self.set_state(id, State::NotActivated);
                    false
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
            self.again(id, &run, asked);
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
                    self.launch(id, &profile, current, None, None)
                        .map_err(|e| vec![e])
                })
        } else {
            // Still Starting or Running: the state goes back to
            // Starting through the launch.
            if let Some(partition) = self.partitions.get_mut(&id) {
                partition.state = State::NotActivated;
            }
            self.launch(
                id,
                &run.profile,
                run.current.clone(),
                Some(IplSource::Recorded),
                run.keylock,
            )
            .map_err(|e| vec![e])
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
            let active = self.partitions.get(&next).is_some_and(|p| p.state.active());
            if active {
                self.begin_end(next, false);
                return;
            }
            if let Some(queue) = &mut self.ending {
                queue.remove(0);
            }
        }
    }

    /// What time does: a VMM that was asked to end and has not is
    /// killed. Called every second or so.
    pub fn tick(&mut self) {
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
            again: false,
            asked: None,
        });
        partition.last_profile.clone_from(&record.profile);
        partition.state = State::Starting;
        self.sequence = self.sequence.max(record.sequence + 1);
        for line in report {
            self.report(id, line, true);
        }
        let code = &refcode::PARTITION_ADOPTED;
        self.code(id, code.code, &code.text_with(&name), true);
    }
}
