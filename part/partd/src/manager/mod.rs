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
//! - [`service`]: the service channel: the system's codes and states,
//!   shut down asked of it, holding, pings.
//! - [`console`]: the console port and its viewer.
//! - [`panel`]: the control panel functions.
//! - [`dumps`]: dumps taken from outside.

mod config;
mod console;
mod drives;
mod dumps;
mod life;
mod panel;
mod physio;
mod profiles;
mod requests;
mod service;

pub use console::VIEWER_QUEUE;
pub use drives::DriveFacts;
pub use dumps::valid_name as valid_dump_name;
pub use life::code_of;
pub use physio::PhysioFacts;
pub use service::{Answer, Port};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_service;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use model::{Definition, IplType, Keylock, Profile};
use poold::proto::{Given, Want};
use qvsmsg::Issued;
use resource::cgroup::Step;
use resource::{Current, Held, Host};
use vswitch::port::PortSpec;

use crate::proto::{DumpInfo, Event, PanelArgs, Posted, Source, State, Status};
use crate::store::{Kept, Store};

/// The reference codes kept of each partition, in memory and on disk.
pub const HISTORY: usize = 256;
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
    /// Its state when it was last written: rewritten at each change.
    pub state: State,
    /// Its system was found to have no service channel.
    pub no_channel: bool,
    /// The host's devices it holds, by location code.
    pub devices: Vec<String>,
    /// The physical tape drives it holds: each slot and location code.
    pub drives: Vec<(u32, String)>,
}
ipc::wire_struct!(Record {
    id,
    profile,
    current,
    pid,
    sequence,
    state,
    no_channel,
    devices,
    drives
});

/// A dump to be written: where, and what its header says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpOrder {
    /// Its file's name in the partition's dumps directory.
    pub name: String,
    /// The current reference code and its text.
    pub code: String,
    /// The code's text.
    pub text: String,
    /// When it was asked for.
    pub time: u64,
    /// Why: one word.
    pub reason: String,
}

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
    /// Post a reference code through PID 1 with what it is about.
    fn post_about(&mut self, code: &str, text: &str, about: &[refcode::about::About]) {
        let _ = about;
        self.post(code, text);
    }
    /// Keep what a running partition is, or forget it.
    fn record(&mut self, id: u32, record: Option<&Record>);
    /// A line for the host's log.
    fn log(&mut self, line: &str);
    /// Whether a backing store is there: a file or a block device.
    fn exists(&self, backing: &str) -> bool;
    /// Have the pool service give the partition `id` what `wants`
    /// names, held for it: each open file is kept by the machine until
    /// the VMM is started with it, or the medium loaded. What each is.
    fn hold(&mut self, id: u32, wants: &[Want]) -> Result<Vec<Given>, Vec<Issued>>;
    /// Give back what `id` holds: `wants`, or everything when empty.
    fn give_back(&mut self, id: u32, wants: &[Want]);
    /// Send the VMM of `id` a line on its control socket, with the
    /// file held for `with`. Its answer.
    fn control(&mut self, id: u32, line: &str, with: Option<&Want>) -> Result<String, String>;
    /// Whether a host path may back a unit: in a test image only.
    fn host_paths(&self) -> bool;
    /// Delete virtual disks, each its pool and its name: the messages
    /// of those that were not.
    fn delete_vdisks(&mut self, disks: &[(String, String)]) -> Vec<Issued>;
    /// The host's virtual switches; none when the switch service does
    /// not answer.
    fn switches(&mut self) -> Option<Vec<vswitch::Switch>>;
    /// Have the switch service open the port of one of `id`'s adapters:
    /// its tap's open file is kept by the machine until the VMM is
    /// started with it, after the pool service's files. The switch's MTU.
    fn open_port(&mut self, id: u32, spec: &PortSpec) -> Result<u32, Vec<Issued>>;
    /// Close every port of `id`.
    fn close_ports(&mut self, id: u32);
    /// Close every port of a partition not in `running`: at the start.
    fn reconcile_ports(&mut self, running: &[u32]);
    /// Make the listening sockets of `id`'s service channel, named, for
    /// its next VMM: kept by the machine until the VMM is started with
    /// them, after the taps. The VMM's `[[port]]`s, `first` the
    /// descriptor of the first.
    fn channel_sockets(
        &mut self,
        id: u32,
        first: usize,
    ) -> Result<Vec<qsvmm::config::ConsolePort>, String>;
    /// Send a frame to the system of `id` on `port`. It may be dropped
    /// when the system does not take what it is sent.
    fn send(&mut self, id: u32, port: Port, frame: Vec<u8>);
    /// Close the connection of `port`: it is made again after a while.
    fn close_channel(&mut self, id: u32, port: Port);
    /// Have the VMM of `id` keep the partition: its processors stopped,
    /// its memory kept.
    fn keep(&mut self, id: u32) -> Result<(), String>;
    /// The dumps directory of `id` in the pool that holds the media
    /// repository; none when no pool does.
    fn dump_dir(&mut self, id: u32) -> Option<PathBuf>;
    /// Begin a dump of `id`'s VMM into its dumps directory. It is
    /// written in the background, and [`Manager::dumped`] is told how
    /// it went.
    fn begin_dump(&mut self, id: u32, order: &DumpOrder) -> Result<(), String>;
    /// The dumps of `id`, oldest first.
    fn dumps(&mut self, id: u32) -> Vec<DumpInfo>;
    /// Remove a dump of `id`; whether it was there.
    fn remove_dump(&mut self, id: u32, name: &str) -> bool;
    /// Keep `id`'s history on disk, whole.
    fn keep_history(&mut self, id: u32, history: &[Posted]);
    /// `id`'s history as it was kept.
    fn kept_history(&mut self, id: u32) -> Vec<Posted>;
    /// The host's PCI devices now, its IOMMU and what it uses.
    fn physio_facts(&mut self) -> PhysioFacts;
    /// Give the function at `address` to the pass-through driver, or
    /// back to the host's drivers.
    fn bind(&mut self, address: &str, passthrough: bool) -> Result<(), String>;
    /// Reset the function at `address` alone.
    fn reset(&mut self, address: &str) -> Result<(), String>;
    /// Open the VFIO container and the isolation groups `groups` for
    /// `id`'s next VMM, kept by the machine until the VMM is started
    /// with them, after the service channel's sockets: `first` is the
    /// container's descriptor. The container's path and each group's.
    fn open_vfio(
        &mut self,
        id: u32,
        groups: &[u32],
        first: usize,
    ) -> Result<(String, Vec<(u32, String)>), String>;
    /// Keep the fenced devices for a manager started again before the
    /// host restarts.
    fn keep_fenced(&mut self, fenced: &[String]);
    /// The devices fenced since the host's IPL.
    fn fenced(&mut self) -> Vec<String>;
    /// The host's physical tape drives; whether a cartridge is in each
    /// is asked of those not in `held`.
    fn tape_drives(&mut self, held: &[String]) -> Vec<DriveFacts>;
    /// Open the drive at `location` (its SCSI generic device `sg`) for
    /// `id`'s next VMM with a relay to it, kept by the machine until the
    /// VMM is started with it, after the VFIO descriptors: `fd` is its
    /// descriptor there. Its path.
    fn open_drive(
        &mut self,
        id: u32,
        location: &str,
        sg: &str,
        fd: usize,
    ) -> Result<String, String>;
}

/// How a shut down asked of the system goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Delayed or operating system.
    pub(crate) kind: crate::proto::Shutdown,
    /// Activated again afterwards.
    pub(crate) restart: bool,
    /// When it is forced.
    pub(crate) deadline: u64,
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
    /// The IPL type of this activation only.
    pub(crate) ipl_type: Option<IplType>,
    /// Whether to activate it again when it has ended: a restart the
    /// partition manager was asked for.
    pub(crate) again: bool,
    /// When it was asked to end; it is killed a while after.
    pub(crate) asked: Option<u64>,
    /// A shut down the system accepted, and when it is forced.
    pub(crate) plan: Option<Plan>,
    /// Each optical and tape device's slot and the medium it has now.
    pub(crate) media: Vec<(u32, Option<String>)>,
    /// A storage pool it uses that failed: it was ended for that.
    pub(crate) lost: Option<String>,
    /// Each Ethernet adapter's slot and whether its port was opened.
    pub(crate) ports: Vec<(u32, bool)>,
    /// Held with its memory: its processors are stopped.
    pub(crate) kept: bool,
    /// The switches of its adapters whose uplink is down.
    pub(crate) uplinks_down: Vec<String>,
    /// The host's devices it holds, by location code.
    pub(crate) devices: Vec<String>,
    /// The physical tape drives it holds: each slot and location code.
    pub(crate) drives: Vec<(u32, String)>,
}

/// One partition.
pub(crate) struct Partition {
    pub(crate) definition: Definition,
    pub(crate) profiles: BTreeMap<String, Profile>,
    pub(crate) state: State,
    pub(crate) history: VecDeque<Posted>,
    /// The current reference code (`docs/SERVICE.md`).
    pub(crate) current: Option<Posted>,
    pub(crate) run: Option<Run>,
    pub(crate) last_profile: String,
    pub(crate) last_end: String,
    pub(crate) message: Option<Issued>,
    /// The service channel of this activation.
    pub(crate) service: service::Service,
    /// The console port.
    pub(crate) console: console_port::Console,
    /// The viewer of the console and where its client is sent to.
    pub(crate) viewer: Option<(u64, std::sync::mpsc::SyncSender<crate::proto::ConsoleOut>)>,
    /// Function 02's selection for the next activation.
    pub(crate) next: PanelArgs,
    /// The dump being taken, and whether the IPL restarts after it.
    pub(crate) dumping: Option<(String, bool)>,
}

impl Partition {
    fn of(kept: Kept) -> Self {
        Self {
            definition: kept.definition,
            profiles: kept.profiles,
            state: State::NotActivated,
            history: VecDeque::new(),
            current: None,
            run: None,
            last_profile: String::new(),
            last_end: String::new(),
            message: None,
            service: service::Service::default(),
            console: console_port::Console::new(),
            viewer: None,
            next: PanelArgs::default(),
            dumping: None,
        }
    }

    /// Whether it has a VMM: it holds what it was given.
    pub(crate) fn active(&self) -> bool {
        self.run.is_some()
    }

    fn status(&self) -> Status {
        let current = self.current.as_ref();
        Status {
            id: self.definition.id,
            name: self.definition.name.clone(),
            state: self.state,
            code: current.map(|p| p.code.clone()).unwrap_or_default(),
            text: current.map(|p| p.text.clone()).unwrap_or_default(),
            current: self.run.as_ref().map(|r| r.current.clone()),
            profile: self.last_profile.clone(),
            last_end: self.last_end.clone(),
            message: self.message.clone(),
            pid: self.run.as_ref().map_or(0, |r| r.pid),
            ethernet: self.ethernet(),
            service: self.service.facts.clone(),
        }
    }

    /// Its adapters: the running profile's with their link, or the
    /// default profile's.
    fn ethernet(&self) -> Vec<crate::proto::Ethernet> {
        let name = match &self.run {
            Some(r) => &r.profile,
            None => &self.definition.default_profile,
        };
        let Some(profile) = self.profiles.get(name) else {
            return Vec::new();
        };
        let mut out: Vec<crate::proto::Ethernet> = profile
            .ethernet
            .iter()
            .map(|a| crate::proto::Ethernet {
                slot: a.slot,
                switch: a.switch.clone().unwrap_or_default(),
                mac: a.mac.clone().unwrap_or_default(),
                link: self
                    .run
                    .as_ref()
                    .is_some_and(|r| r.ports.contains(&(a.slot, true))),
            })
            .collect();
        out.sort_by_key(|e| e.slot);
        out
    }

    /// The last code the system posted, of its history.
    fn last_system(&self) -> Option<&Posted> {
        self.history
            .iter()
            .rev()
            .find(|p| p.source == Source::System)
    }
}

impl Posted {
    /// A note of the history, never where the partition is: the
    /// channel's progress, and a partition found running.
    pub(crate) fn is_note(&self) -> bool {
        self.source == Source::Host
            && (self.code.starts_with("C2009") || self.code == refcode::PARTITION_ADOPTED.code)
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
    /// The next number for a request asked of a system, or a viewer.
    pub(crate) next_tag: u64,
    /// The devices released for partitions.
    pub(crate) released: pio::Released,
    /// The devices fenced since the host's IPL, by location code.
    pub(crate) fenced: Vec<String>,
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
            .map(|k| {
                let id = k.definition.id;
                let mut p = Partition::of(k);
                // The history kept, its last code the current one.
                let mut history: VecDeque<Posted> = system.kept_history(id).into();
                while history.len() > HISTORY {
                    history.pop_front();
                }
                p.current = history.iter().rev().find(|c| !c.is_note()).cloned();
                p.history = history;
                (id, p)
            })
            .collect();
        let released = store.released().unwrap_or_else(|reason| {
            system.log(&format!("refused: {reason}"));
            system.post(
                refcode::PARTITION_STATE_INVALID.code,
                &refcode::PARTITION_STATE_INVALID.text_with(&reason),
            );
            pio::Released::default()
        });
        let fenced = system.fenced();
        Self {
            system,
            store,
            host,
            host_serial: host_serial.to_string(),
            partitions,
            subscribers: Vec::new(),
            sequence: 1,
            ending: None,
            next_tag: 1,
            released,
            fenced,
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
            Event::Profile {
                name,
                profile,
                revision,
                default,
                ..
            } => match revision {
                Some(r) => format!(
                    "{name}: profile {profile} at revision {r}{}",
                    if *default { ", the default" } else { "" }
                ),
                None => format!("{name}: profile {profile} deleted"),
            },
            Event::Channel { name, service, .. } => format!(
                "{name}: service channel {}{}",
                if service.up { "up" } else { "down" },
                if service.up && !service.responding {
                    ", not responding"
                } else {
                    ""
                }
            ),
            Event::Dump {
                name, dump, error, ..
            } => match error {
                None => format!("{name}: dump {dump} written"),
                Some(e) => format!("{name}: dump {dump} not written: {e}"),
            },
            Event::TapeDrive {
                location, drive, ..
            } => match drive {
                Some(d) if d.holder != 0 => format!("tape drive {location}: held by {}", d.name),
                Some(d) if d.released => format!("tape drive {location}: released"),
                Some(_) => format!("tape drive {location}: the host's"),
                None => format!("tape drive {location}: not on the host"),
            },
            Event::Entitlement {
                name, current, by, ..
            } => format!(
                "{name}: entitled to {} processing units, by {by}",
                current.processors.units()
            ),
            Event::Device {
                location,
                change,
                name,
                ..
            } => {
                if name.is_empty() {
                    format!("device {location}: {}", change.word())
                } else {
                    format!("device {location}: {}, partition {name}", change.word())
                }
            }
        };
        self.system.log(&line);
        self.subscribers.retain(|s| s.send(event.clone()).is_ok());
    }

    /// Add a subscriber: every event from now on is sent to it.
    pub fn subscribe(&mut self, subscriber: Sender<Event>) {
        self.subscribers.push(subscriber);
    }

    /// What runs is recorded with its state, for a manager started
    /// after this one.
    pub(crate) fn write_record(&mut self, id: u32) {
        let Some(partition) = self.partitions.get(&id) else {
            return;
        };
        if let Some(run) = &partition.run {
            let record = Record {
                id,
                profile: run.profile.clone(),
                current: run.current.clone(),
                pid: run.pid,
                sequence: run.sequence,
                state: partition.state,
                no_channel: partition.service.facts.none,
                devices: run.devices.clone(),
                drives: run.drives.clone(),
            };
            self.system.record(id, Some(&record));
        }
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
        self.write_record(id);
        self.publish(&Event::State {
            id,
            name,
            state,
            time,
        });
    }

    /// Add a host reference code to a partition's history and say so.
    /// With `post`, PID 1 is given it too.
    pub(crate) fn code(&mut self, id: u32, code: &str, text: &str, post: bool) {
        let posted = Posted {
            time: self.system.now(),
            code: code.to_string(),
            text: text.to_string(),
            source: Source::Host,
            words: Vec::new(),
        };
        if post {
            let about = self.about(id);
            self.system.post_about(code, text, &about);
        }
        self.add(id, posted, post);
    }

    /// What a code of partition `id` is about: the partition.
    pub(crate) fn about(&self, id: u32) -> Vec<refcode::about::About> {
        self.partitions.get(&id).map_or_else(Vec::new, |p| {
            vec![refcode::about::partition(
                id,
                &p.definition.name,
                &p.definition.serial,
            )]
        })
    }

    /// Add `posted` to `id`'s history, keep it, make it the current
    /// code when the rule says (`docs/SERVICE.md`), and say so when
    /// `publish`.
    pub(crate) fn add(&mut self, id: u32, posted: Posted, publish: bool) {
        let Some(partition) = self.partitions.get_mut(&id) else {
            return;
        };
        if partition.history.len() >= HISTORY {
            partition.history.pop_front();
        }
        partition.history.push_back(posted.clone());
        let condition = posted.code.starts_with('A') || posted.code.starts_with('B');
        // A note of the channel's progress is the history's, not where
        // the partition is.
        let note = posted.is_note();
        let current = match posted.source {
            // A note leaves the current code as it is, but for a
            // condition of the host's it ends: the system's again.
            Source::Host if note => partition
                .current
                .as_ref()
                .filter(|c| {
                    c.source == Source::Host && (c.code.starts_with('A') || c.code.starts_with('B'))
                })
                .and_then(|_| partition.last_system().cloned()),
            Source::System => Some(posted.clone()),
            // The host's own while the system has posted none, after
            // its VMM has ended, and for a condition of the host's.
            Source::Host
                if !partition.service.system_posted || !partition.active() || condition =>
            {
                Some(posted.clone())
            }
            // A host code of progress while the system posts: the
            // current code is the system's again.
            Source::Host => partition.last_system().cloned(),
        };
        if current.is_some() {
            partition.current = current;
        }
        let history: Vec<Posted> = partition.history.iter().cloned().collect();
        let name = partition.definition.name.clone();
        self.system.keep_history(id, &history);
        if publish {
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

    /// A number of its own for a request asked of a system or a viewer.
    pub(crate) fn tag(&mut self) -> u64 {
        let tag = self.next_tag;
        self.next_tag += 1;
        tag
    }

    /// The partition `id`'s name, or its number when it has none.
    pub(crate) fn name_of(&self, id: u32) -> String {
        self.partitions
            .get(&id)
            .map_or_else(|| id.to_string(), |p| p.definition.name.clone())
    }
}
