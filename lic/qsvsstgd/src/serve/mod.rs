// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service: it finds the load-source disks, assembles the mirrored
//! set their label describes, serves it as a ublk device that outlives
//! this process, keeps the label in step with the members, brings a
//! stale member back in step, and prepares a disk that is added or put
//! in a member's place. With an unprotected load source there is no
//! set: it names the data partition and answers status.

mod guard;
mod journal;
mod prepare;

use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use blockdev::{BlockDev, LinuxBlockDev};
use hostdisk::disks::{self, LoadSource};
use hostdisk::set::{BLOCK, Member, MemberState, SetLabel};
use protect::UnitState;
use protect::set::{MirrorMember, MirrorSet, Resume, SetEvent};
use qsvsstgd::{DATA_DEVICE, MIRRORED, NONE, Request, SOCKET, Status, Storage, Unit, dispatch};
use refcode::post::remote;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use supervise::signal::{Signals, TERM};
use ublk::{Device, Options};

use guard::{Guarded, Recorded};
use journal::Journal;

/// The device number this boot's set has: a server that starts and
/// finds it takes the device over instead of adding one.
const DEVICE_ID: &str = "/run/qsvsstgd.id";
/// How long the load-source disks may take to appear.
const DISK_WAIT: Duration = Duration::from_secs(10);
/// Megabytes copied between two looks at the socket and the signals.
const STEP_CHUNKS: u64 = 16;
/// How long a copy stands back, after each megabyte, while the host
/// has transfers of its own under way.
const GIVE_WAY: Duration = Duration::from_millis(20);

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Why a member is being copied to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// It was stale: an unclean stop, or it was away.
    Resync,
    /// It is a replacement.
    Rebuild,
}

struct Work {
    kind: Kind,
    member: usize,
    resume: Resume,
}

/// A request that was accepted and is carried out by the loop.
enum Job {
    Start(String),
    Replace(usize, String),
}

/// What the socket's thread asks the loop.
enum Msg {
    Status(Sender<Status>),
    Start(String, Sender<Result<(), String>>),
    Replace(u32, String, Sender<Result<(), String>>),
}

fn unit_state(state: MemberState) -> UnitState {
    match state {
        MemberState::Active => UnitState::Active,
        MemberState::Suspended => UnitState::Suspended,
        MemberState::Resuming => UnitState::Resuming,
        MemberState::Failed => UnitState::NotOperational,
        MemberState::Missing => UnitState::Missing,
    }
}

fn member_state(state: UnitState) -> MemberState {
    match state {
        UnitState::Active | UnitState::Unprotected => MemberState::Active,
        UnitState::Suspended => MemberState::Suspended,
        UnitState::Resuming => MemberState::Resuming,
        UnitState::NotOperational => MemberState::Failed,
        UnitState::Missing => MemberState::Missing,
    }
}

/// The load-source disks, waited for. A set whose label names a member
/// that is not there yet is given the whole wait: a disk behind a slow
/// controller is late, not missing, and taking it for missing costs a
/// resynchronization.
fn found_disks() -> Vec<LoadSource> {
    let give_up = Instant::now() + DISK_WAIT;
    loop {
        let sources = disks::load_sources_within(Duration::from_secs(1));
        let whole = disks::label(&sources).is_none_or(|label| {
            label
                .members
                .iter()
                .filter(|m| m.state != MemberState::Missing)
                .all(|m| sources.iter().any(|s| s.data_guid == m.partition))
        });
        if (!sources.is_empty() && whole) || Instant::now() >= give_up {
            return sources;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The service's state.
struct Core {
    sources: Vec<LoadSource>,
    /// The inventory's disks: serial numbers and location codes.
    inventory: Vec<(String, String, String)>,
    label: Option<SetLabel>,
    set: Option<MirrorSet>,
    /// The states the label on the disks has, for the device's transfers.
    recorded: Arc<Recorded>,
    device: Option<Device>,
    /// What `/data` is mounted from.
    data: Option<PathBuf>,
    work: Option<Work>,
    job: Option<Job>,
    restart_needed: bool,
    failure: String,
}

impl Core {
    /// The serial number and location code of `disk`.
    fn identity(&self, disk: &str) -> (String, String) {
        self.inventory
            .iter()
            .find(|(name, _, _)| name == disk)
            .map(|(_, serial, location)| (serial.clone(), location.clone()))
            .unwrap_or_default()
    }

    fn read_inventory(&mut self) {
        self.inventory = hostinv::read(&hostinv::Roots::default(), None)
            .disks
            .into_iter()
            .map(|d| (d.name, d.serial, d.location))
            .collect();
    }

    /// The load-source disk that holds a member.
    fn source_of(&self, member: &Member) -> Option<&LoadSource> {
        self.sources
            .iter()
            .find(|s| s.data_guid == member.partition)
    }

    /// Find the disks and assemble what their label describes.
    fn assemble() -> Result<Self, String> {
        let mut core = Core {
            sources: found_disks(),
            inventory: Vec::new(),
            label: None,
            set: None,
            recorded: Arc::default(),
            device: None,
            data: None,
            work: None,
            job: None,
            restart_needed: false,
            failure: String::new(),
        };
        core.read_inventory();
        let Some(mut label) = disks::label(&core.sources) else {
            // Unprotected: the partition itself.
            core.data = core.sources.first().map(|s| s.data.clone());
            return Ok(core);
        };
        // A device of this boot is still there: this server takes it
        // over, and nothing was cut off but the server.
        let kept = fs::read_to_string(DEVICE_ID)
            .ok()
            .and_then(|id| id.trim().parse::<u32>().ok())
            .filter(|id| Path::new(&format!("/dev/ublkb{id}")).exists());
        let unclean = !label.clean && kept.is_none();
        let attached = |m: &Member| core.source_of(m).is_some();
        let source = label
            .members
            .iter()
            .position(|m| m.state == MemberState::Active && attached(m));
        let mut members = Vec::new();
        for (i, m) in label.members.iter().enumerate() {
            let dev: Option<Box<dyn BlockDev>> = match core.source_of(m) {
                Some(s) => Some(Box::new(LinuxBlockDev::open(&s.data).map_err(text)?)),
                None => None,
            };
            let state = match m.state {
                // The members may differ after an unclean stop: the
                // first that was in step is the one the others follow.
                MemberState::Active if unclean && Some(i) != source => UnitState::Suspended,
                other => unit_state(other),
            };
            members.push(MirrorMember {
                uuid: *m.partition.as_bytes(),
                state,
                dev,
            });
        }
        let set = MirrorSet::open(*label.id.as_bytes(), label.blocks, members).map_err(text)?;
        // The label as read is what the disks say of the members now.
        core.recorded.set(guard::states(&set));
        let target = Arc::new(Guarded::new(set.clone(), Arc::clone(&core.recorded)));
        let device = match kept {
            Some(id) => Device::recover(id, target),
            None => Device::start_with(target, Options { recovery: true }),
        }
        .map_err(text)?;
        let _ = fs::write(DEVICE_ID, device.id().to_string());
        let node = device.block_path();
        let give_up = Instant::now() + Duration::from_secs(10);
        while !node.exists() {
            if Instant::now() > give_up {
                return Err(format!("{} did not appear", node.display()));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        label.clean = false;
        core.data = Some(node.clone());
        core.label = Some(label);
        core.set = Some(set);
        core.device = Some(device);
        core.write_label(true);
        let what = if kept.is_some() {
            "taken over"
        } else {
            "served"
        };
        remote::code(
            &refcode::SET_ASSEMBLED,
            &format!("{} units, {what} as {}", core.units().len(), node.display()),
        );
        for unit in core.units() {
            if unit.state == "Missing" || unit.state == "Failed" {
                Core::post_failed(&unit);
            }
        }
        Ok(core)
    }

    fn post_failed(unit: &Unit) {
        let place = if unit.location.is_empty() {
            String::new()
        } else {
            format!(" at {}", unit.location)
        };
        remote::code(
            &refcode::UNIT_FAILED,
            &format!(
                "unit {}{place}, serial {}: {}",
                unit.number,
                if unit.serial.is_empty() {
                    "unknown"
                } else {
                    &unit.serial
                },
                unit.state.to_lowercase()
            ),
        );
    }

    /// Bring the label in step with the set's members and write it to
    /// every disk that is there, when it changed or `force`.
    fn write_label(&mut self, force: bool) {
        let (Some(set), Some(mut label)) = (&self.set, self.label.clone()) else {
            return;
        };
        for (m, now) in label.members.iter_mut().zip(set.members()) {
            m.state = member_state(now.state);
            m.partition = uuid::Uuid::from_bytes(now.uuid);
        }
        for m in &mut label.members {
            let disk = self.source_of(m).map(|s| s.disk.clone());
            if let Some(disk) = disk {
                let (serial, location) = self.identity(&disk);
                m.serial = serial;
                m.location = location;
            }
        }
        if !force && self.label.as_ref() == Some(&label) {
            self.recorded.set(guard::states(set));
            return;
        }
        let written = disks::labels(&self.sources)
            .map_err(text)
            .and_then(|mut l| l.write(&label.encode()).map_err(text));
        if let Err(e) = written {
            eprintln!("the set's label was not written: {e}");
        }
        // Transfers that waited for this go on.
        self.recorded.set(guard::states(set));
        self.label = Some(label);
    }

    /// The units, for status and for the codes.
    fn units(&self) -> Vec<Unit> {
        let Some(label) = &self.label else {
            return self
                .sources
                .iter()
                .take(1)
                .map(|s| {
                    let (serial, location) = self.identity(&s.disk);
                    Unit {
                        number: 1,
                        disk: s.disk.clone(),
                        serial,
                        location,
                        bytes: disks::disk_bytes(&s.disk),
                        state: "Unprotected".into(),
                    }
                })
                .collect();
        };
        let now = self.set.as_ref().map(MirrorSet::members);
        label
            .members
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let disk = self
                    .source_of(m)
                    .map(|s| s.disk.clone())
                    .unwrap_or_default();
                let state = now
                    .as_ref()
                    .and_then(|n| n.get(i))
                    .map_or(m.state, |n| member_state(n.state));
                Unit {
                    number: u32::try_from(i + 1).unwrap_or(u32::MAX),
                    bytes: disks::disk_bytes(&disk),
                    disk,
                    serial: m.serial.clone(),
                    location: m.location.clone(),
                    state: state.word().into(),
                }
            })
            .collect()
    }

    fn status(&self) -> Status {
        let (work, work_unit, percent) = match (&self.work, &self.job) {
            (Some(w), _) => (
                match w.kind {
                    Kind::Resync => "resynchronizing",
                    Kind::Rebuild => "rebuilding",
                },
                w.member + 1,
                u32::from(w.resume.progress().percent),
            ),
            (None, Some(Job::Replace(member, _))) => ("preparing", member + 1, 0),
            (None, Some(Job::Start(_))) => ("preparing", 2, 0),
            (None, None) => ("", 0, 0),
        };
        Status {
            protection: if self.label.is_some() { MIRRORED } else { NONE }.into(),
            device: self
                .data
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            units: self.units(),
            work: work.into(),
            work_unit: u32::try_from(work_unit).unwrap_or(0),
            percent,
            restart_needed: self.restart_needed,
            failure: self.failure.clone(),
        }
    }

    /// What the set did by itself since the last look.
    fn events(&mut self) {
        let Some(set) = &self.set else { return };
        let events = set.drain_events();
        // Every look brings the label level: a transfer that changed a
        // member's state waits for it.
        self.write_label(false);
        if events.is_empty() {
            return;
        }
        let units = self.units();
        for event in events {
            match event {
                SetEvent::Failed { member, what } => {
                    eprintln!("unit {} failed: {what}", member + 1);
                    if let Some(unit) = units.get(member) {
                        Core::post_failed(unit);
                    }
                    if self.work.as_ref().is_some_and(|w| w.member == member) {
                        self.work = None;
                    }
                }
                SetEvent::Healed { member, lba } => {
                    println!(
                        "unit {}: block {lba} written again from the other",
                        member + 1
                    );
                }
                // At open: said with the units, and brought back below.
                SetEvent::Missing { .. } | SetEvent::Stale { .. } => {}
            }
        }
    }

    /// Start copying to `member`.
    fn begin(&mut self, member: usize, kind: Kind) {
        let Some(set) = &self.set else { return };
        let mut journal = Journal::on(&self.sources);
        match set.resume(member, &mut journal) {
            Ok(resume) => {
                let code = match kind {
                    Kind::Resync => &refcode::RESYNC_STARTED,
                    Kind::Rebuild => &refcode::REBUILD_STARTED,
                };
                let p = resume.progress();
                remote::code(
                    code,
                    &format!(
                        "unit {}, {} of {} MiB already in step",
                        member + 1,
                        p.chunks_done,
                        p.chunks
                    ),
                );
                self.work = Some(Work {
                    kind,
                    member,
                    resume,
                });
                self.write_label(false);
            }
            Err(e) => {
                self.failure = format!("unit {}: {e}", member + 1);
                remote::code(&refcode::REBUILD_FAILED, &self.failure);
            }
        }
    }

    /// Copy a step of the work under way; start what is waiting.
    fn work(&mut self) {
        let Some(set) = self.set.clone() else { return };
        if self.work.is_none() && self.job.is_none() {
            // A member that is there and not in step is brought back.
            let stale = set.members().iter().position(|m| {
                m.attached
                    && matches!(
                        m.state,
                        UnitState::Suspended | UnitState::Resuming | UnitState::NotOperational
                    )
            });
            // One that failed is tried once an IPL, not for ever.
            let failed = self.failure.is_empty();
            if let Some(member) = stale.filter(|_| failed) {
                self.begin(member, Kind::Resync);
            }
        }
        let Some(work) = &mut self.work else { return };
        let mut journal = Journal::on(&self.sources);
        // A megabyte at a time: each is copied with the set held, and
        // the host's own transfers go between them, not behind them all.
        let mut step = set.resume_step(&mut work.resume, &mut journal, 1);
        for _ in 1..STEP_CHUNKS {
            if !matches!(&step, Ok(progress) if !progress.complete) {
                break;
            }
            let give_up = Instant::now() + GIVE_WAY;
            while self.recorded.busy() && Instant::now() < give_up {
                std::thread::sleep(Duration::from_micros(200));
            }
            step = set.resume_step(&mut work.resume, &mut journal, 1);
        }
        match step {
            Ok(progress) if progress.complete => {
                let (code, member) = match work.kind {
                    Kind::Resync => (&refcode::RESYNC_FINISHED, work.member),
                    Kind::Rebuild => (&refcode::REBUILD_FINISHED, work.member),
                };
                self.work = None;
                self.write_label(false);
                remote::code(
                    code,
                    &format!("unit {}, {} MiB", member + 1, progress.chunks),
                );
            }
            Ok(_) => {}
            Err(e) => {
                self.failure = format!("unit {}: {e}", work.member + 1);
                self.work = None;
                self.write_label(false);
                remote::code(&refcode::REBUILD_FAILED, &self.failure);
            }
        }
    }

    /// Carry out the request that was accepted.
    fn job(&mut self) {
        let Some(job) = self.job.take() else { return };
        self.failure.clear();
        let done = match job {
            Job::Start(disk) => prepare::start_protection(self, &disk),
            Job::Replace(member, disk) => prepare::replace(self, member, &disk),
        };
        if let Err(e) = done {
            eprintln!("the disk was not taken: {e}");
            self.failure = e;
        }
    }

    /// End in order: the device stopped, the label marked clean.
    fn stop(mut self) -> ! {
        let stopped = self.device.take().map_or(Ok(()), Device::stop);
        match stopped {
            Ok(()) => {
                if let Some(label) = &mut self.label {
                    label.clean = true;
                }
                self.write_label(true);
                let _ = fs::remove_file(DEVICE_ID);
            }
            Err(e) => eprintln!("the device was not stopped: {e}"),
        }
        std::process::exit(0);
    }
}

/// The socket's side of a request: ask the loop and wait.
struct Asker(Sender<Msg>);

impl Asker {
    fn ask<T>(&self, msg: impl FnOnce(Sender<T>) -> Msg) -> Option<T> {
        let (tx, rx) = mpsc::channel();
        self.0.send(msg(tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(2)).ok()
    }
}

impl Storage for Asker {
    fn status(&mut self) -> Status {
        self.ask(Msg::Status).unwrap_or_default()
    }

    fn start_protection(&mut self, disk: String) -> Result<(), String> {
        self.ask(|tx| Msg::Start(disk, tx))
            .unwrap_or_else(|| Err("the storage service is busy".into()))
    }

    fn replace(&mut self, number: u32, disk: String) -> Result<(), String> {
        self.ask(|tx| Msg::Replace(number, disk, tx))
            .unwrap_or_else(|| Err("the storage service is busy".into()))
    }
}

fn serve_socket(listener: &UnixListener, tx: &Sender<Msg>) {
    let mut asker = Asker(tx.clone());
    for stream in listener.incoming().flatten() {
        let _ = supervise::control::answer(stream, |request| {
            Ok(dispatch(&mut asker, ipc::decode::<Request>(request)?))
        });
    }
}

fn answer(core: &mut Core, rx: &Receiver<Msg>) {
    while let Ok(msg) = rx.try_recv() {
        match msg {
            Msg::Status(reply) => {
                let _ = reply.send(core.status());
            }
            Msg::Start(disk, reply) => {
                let checked = prepare::check_start(core, &disk);
                if checked.is_ok() {
                    core.job = Some(Job::Start(disk));
                }
                let _ = reply.send(checked);
            }
            Msg::Replace(number, disk, reply) => {
                let checked = prepare::check_replace(core, number, &disk);
                if let Ok(member) = checked {
                    core.job = Some(Job::Replace(member, disk));
                }
                let _ = reply.send(checked.map(|_| ()));
            }
        }
    }
}

/// Whether `SIGTERM` came within `wait`.
fn term(signals: &mut Signals, wait: Duration) -> bool {
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    let mut fds = [PollFd::from_borrowed_fd(signals.fd(), PollFlags::IN)];
    let readable = poll(&mut fds, Some(&timeout)).is_ok() && !fds[0].revents().is_empty();
    readable && signals.read().contains(&TERM)
}

/// Run the storage service. It returns only when it could not start.
pub fn run() -> String {
    let mut signals = match Signals::take(&[TERM]) {
        Ok(signals) => signals,
        Err(e) => return format!("the signal descriptor: {e}"),
    };
    let mut core = match Core::assemble() {
        Ok(core) => core,
        Err(e) => {
            remote::code(&refcode::SET_NOT_ASSEMBLED, &e);
            let _ = fs::write(DATA_DEVICE, "");
            return e;
        }
    };
    let data = core
        .data
        .as_ref()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    if let Err(e) = fs::write(DATA_DEVICE, &data) {
        return format!("{DATA_DEVICE}: {e}");
    }
    let (tx, rx) = mpsc::channel();
    let _ = fs::remove_file(SOCKET);
    let listener = match UnixListener::bind(SOCKET) {
        Ok(listener) => listener,
        Err(e) => return format!("{SOCKET}: {e}"),
    };
    std::thread::spawn(move || serve_socket(&listener, &tx));
    if let Err(e) = supervise::ready::signal() {
        return format!("the readiness pipe: {e}");
    }
    loop {
        // While a copy runs the loop does not wait.
        let wait = if core.work.is_some() || core.job.is_some() {
            Duration::ZERO
        } else {
            Duration::from_millis(100)
        };
        if term(&mut signals, wait) {
            core.stop();
        }
        answer(&mut core, &rx);
        core.events();
        core.job();
        core.work();
    }
}

/// Blocks of the set over a data partition of `bytes`.
pub(crate) fn blocks_of(bytes: u64) -> u64 {
    bytes / BLOCK
}
