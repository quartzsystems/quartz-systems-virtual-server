// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager itself: every partition's definition,
//! profiles and state, the accounts of what the running ones hold, and
//! what each request and each end of a VMM does to them. It does
//! nothing to the machine itself: what it wants done it asks of a
//! [`System`], so all of it runs in a test.
//!
//! - [`requests`]: the socket's requests.
//! - [`life`]: activation, the VMM's reports, its end, and what
//!   follows each kind of end.
//! - [`config`]: the configuration a VMM is started with.

mod config;
mod life;
mod requests;

pub use life::code_of;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, VecDeque};
use std::sync::mpsc::Sender;

use model::{Definition, Keylock, Profile};
use qvsmsg::Issued;
use resource::cgroup::Step;
use resource::{Current, Held, Host};

use crate::proto::{Event, Posted, State, Status};
use crate::store::{Kept, Store};

/// The reference codes kept of each partition.
pub const HISTORY: usize = 64;
/// The longest line of a VMM's report that is read.
pub const MAX_LINE: usize = 512;
/// Seconds a VMM is given to end when asked, before it is killed.
pub const END_GRACE_SECS: u64 = 10;

/// How a VMM ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// With an exit code.
    Exit(i32),
    /// Killed by a signal.
    Signal(i32),
    /// It is gone and left no word of how: it was killed while nobody
    /// could wait for it.
    Unknown,
}

impl Ended {
    /// `exit <code>` or `signal <number>`.
    pub fn how(self) -> String {
        match self {
            Ended::Exit(code) => format!("exit {code}"),
            Ended::Signal(signal) => format!("signal {signal}"),
            Ended::Unknown => "signal ?".to_string(),
        }
    }
}

/// What a running partition is, for a partition manager started after
/// this one ended: kept in the run directory, which a host's restart
/// empties.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The partition.
    pub id: u32,
    /// The profile it was activated with.
    pub profile: String,
    /// What it holds.
    pub current: Current,
    /// Its VMM's process.
    pub pid: u32,
    /// Its place in the order of activations.
    pub sequence: u64,
}
ipc::wire_struct!(Record {
    id,
    profile,
    current,
    pid,
    sequence
});

/// What the partition manager asks of the machine.
pub trait System {
    /// Seconds since the epoch.
    fn now(&self) -> u64;
    /// Carry out a plan of the cgroup tree.
    fn apply(&mut self, plan: &[Step]) -> Result<(), String>;
    /// Remove a group that has no process left.
    fn remove_group(&mut self, group: &str);
    /// Start the VMM of the partition `id` in `group` with the
    /// configuration `config`. Its process.
    fn launch(&mut self, id: u32, group: &str, config: &str) -> Result<u32, String>;
    /// Ask a VMM to end.
    fn end(&mut self, pid: u32);
    /// Kill every process of `group`.
    fn kill(&mut self, group: &str);
    /// Post a reference code through PID 1.
    fn post(&mut self, code: &str, text: &str);
    /// Keep what a running partition is, or forget it.
    fn record(&mut self, id: u32, record: Option<&Record>);
    /// A line for the host's log.
    fn log(&mut self, line: &str);
    /// Whether a backing store is there: a file or a block device.
    fn exists(&self, backing: &str) -> bool;
}

/// A partition while it is activated.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    pub(crate) current: Current,
    pub(crate) profile: String,
    pub(crate) pid: u32,
    pub(crate) group: String,
    pub(crate) sequence: u64,
    /// The keylock position of this activation only.
    pub(crate) keylock: Option<Keylock>,
    /// Whether to activate it again when it has ended: a restart the
    /// partition manager was asked for.
    pub(crate) again: bool,
    /// When it was asked to end; it is killed a while after.
    pub(crate) asked: Option<u64>,
}

/// One partition.
#[derive(Clone, Debug)]
pub(crate) struct Partition {
    pub(crate) definition: Definition,
    pub(crate) profiles: BTreeMap<String, Profile>,
    pub(crate) state: State,
    pub(crate) history: VecDeque<Posted>,
    pub(crate) run: Option<Run>,
    pub(crate) last_profile: String,
    pub(crate) last_end: String,
    pub(crate) message: Option<Issued>,
}

impl Partition {
    fn of(kept: Kept) -> Self {
        Self {
            definition: kept.definition,
            profiles: kept.profiles,
            state: State::NotActivated,
            history: VecDeque::new(),
            run: None,
            last_profile: String::new(),
            last_end: String::new(),
            message: None,
        }
    }

    fn status(&self) -> Status {
        let last = self.history.back();
        Status {
            id: self.definition.id,
            name: self.definition.name.clone(),
            state: self.state,
            code: last.map(|p| p.code.clone()).unwrap_or_default(),
            text: last.map(|p| p.text.clone()).unwrap_or_default(),
            current: self.run.as_ref().map(|r| r.current.clone()),
            profile: self.last_profile.clone(),
            last_end: self.last_end.clone(),
            message: self.message.clone(),
            pid: self.run.as_ref().map_or(0, |r| r.pid),
        }
    }
}

/// The partition manager.
pub struct Manager<S: System> {
    pub(crate) system: S,
    pub(crate) store: Store,
    pub(crate) host: Host,
    /// The host's serial number: what a new partition's is made from.
    pub(crate) host_serial: String,
    pub(crate) partitions: BTreeMap<u32, Partition>,
    subscribers: Vec<Sender<Event>>,
    /// The next activation's place in their order.
    pub(crate) sequence: u64,
    /// The partitions still to be ended for the host's shutdown, the
    /// next one first. Once it has been asked for, nothing is
    /// activated.
    pub(crate) ending: Option<Vec<u32>>,
}

impl<S: System> Manager<S> {
    /// The manager of the partitions `store` keeps, on `host`.
    /// What state refused is logged and posted.
    pub fn new(mut system: S, store: Store, host: Host, host_serial: &str) -> Self {
        let (kept, refused) = store.load();
        for reason in refused {
            system.log(&format!("refused: {reason}"));
            system.post(
                refcode::PARTITION_STATE_INVALID.code,
                &refcode::PARTITION_STATE_INVALID.text_with(&reason),
            );
        }
        let partitions = kept
            .into_iter()
            .map(|k| (k.definition.id, Partition::of(k)))
            .collect();
        Self {
            system,
            store,
            host,
            host_serial: host_serial.to_string(),
            partitions,
            subscribers: Vec::new(),
            sequence: 1,
            ending: None,
        }
    }

    /// The machine the manager works through.
    pub fn system(&mut self) -> &mut S {
        &mut self.system
    }

    /// What every activated partition holds.
    pub(crate) fn held(&self) -> Vec<Held> {
        self.partitions
            .values()
            .filter_map(|p| {
                p.run.as_ref().map(|r| Held {
                    partition: p.definition.id,
                    current: r.current.clone(),
                })
            })
            .collect()
    }

    /// Send an event to every subscriber that is still there, and say
    /// it in the log.
    pub(crate) fn publish(&mut self, event: &Event) {
        let line = match event {
            Event::State { name, state, .. } => format!("{name}: {}", state.word()),
            Event::Code { name, posted, .. } => {
                format!("{name}: {} {}", posted.code, posted.text)
            }
            Event::Resources { name, current, .. } => match current {
                Some(current) => format!(
                    "{name}: holds {} processing units, {} MiB",
                    current.processors.units(),
                    current.memory_mib
                ),
                None => format!("{name}: holds nothing"),
            },
            Event::Defined { name, exists, .. } => {
                format!("{name}: {}", if *exists { "defined" } else { "deleted" })
            }
        };
        self.system.log(&line);
        self.subscribers.retain(|s| s.send(event.clone()).is_ok());
    }

    /// Add a subscriber: every event from now on is sent to it.
    pub fn subscribe(&mut self, subscriber: Sender<Event>) {
        self.subscribers.push(subscriber);
    }

    /// Set a partition's state and say so.
    pub(crate) fn set_state(&mut self, id: u32, state: State) {
        let time = self.system.now();
        let Some(partition) = self.partitions.get_mut(&id) else {
            return;
        };
        if partition.state == state {
            return;
        }
        partition.state = state;
        let name = partition.definition.name.clone();
        self.publish(&Event::State {
            id,
            name,
            state,
            time,
        });
    }

    /// Add a reference code to a partition's history and say so. With
    /// `post`, PID 1 is given it too.
    pub(crate) fn code(&mut self, id: u32, code: &str, text: &str, post: bool) {
        let time = self.system.now();
        let Some(partition) = self.partitions.get_mut(&id) else {
            return;
        };
        let posted = Posted {
            time,
            code: code.to_string(),
            text: text.to_string(),
        };
        if partition.history.len() >= HISTORY {
            partition.history.pop_front();
        }
        partition.history.push_back(posted.clone());
        let name = partition.definition.name.clone();
        if post {
            self.system.post(code, text);
            self.publish(&Event::Code { id, name, posted });
        }
    }

    /// Every partition's status, in ID order.
    pub fn statuses(&self) -> Vec<Status> {
        self.partitions.values().map(Partition::status).collect()
    }

    /// The base of the cgroup tree for what is held now.
    pub(crate) fn apply_base(&mut self) -> Result<(), String> {
        let plan = resource::cgroup::base(&self.host, &self.held());
        self.system.apply(&plan)
    }

    /// Set the tree up when the manager starts.
    pub fn start(&mut self) {
        if let Err(e) = self.apply_base() {
            self.system.log(&format!("resource controls: {e}"));
            self.system.post(
                refcode::CONTROLS_NOT_SET.code,
                &refcode::CONTROLS_NOT_SET.text_with(&e),
            );
        }
    }
}
