// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The pool service on Linux: at its start it finds the host's pool
//! disks by their labels and assembles each pool; then it serves its
//! socket and, between requests, does each pool's work: a unit that
//! failed taken out of service and a hot spare put in its place, a
//! rebuild or a resynchronization a step at a time, the label kept in
//! step, the pool's fill watched. At the host's shutdown it stops every
//! pool in order.
//!
//! A pool's set is served through ublk with recovery: when the service
//! is stopped by a fault and started again, the device holds the
//! partitions' I/O until the new service takes it over.

mod create;
mod guard;
mod library;
mod records;
mod runtime;

use std::fmt::Write as _;
use std::fs;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use loadsource::disks;
use pool::disk::{self, Kind};
use pool::label::{self, PoolLabel, UnitState};
use pool::names::PoolDiskNames;
use protect::set::{ParityWork, SetEvent};
use refcode::RefCode;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use supervise::signal::{Signals, TERM};

use crate::proto::{self, Disk, PoolStatus};
use runtime::{Found, Pool, Set};

/// One disk, as the host's inventory knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiskIdentity {
    /// Its kernel name.
    pub name: String,
    /// Its serial number.
    pub serial: String,
    /// Its location code.
    pub location: String,
}

/// What the service is given by the product it runs as.
#[derive(Clone, Copy)]
pub struct Host {
    /// What a pool disk carries that is the product's.
    pub names: &'static PoolDiskNames,
    /// What a load-source disk carries: those are never a pool's.
    pub load_source: &'static appliance::LoadSourceNames,
    /// The host's disks: serial numbers and location codes.
    pub inventory: fn() -> Vec<DiskIdentity>,
    /// Post a reference code.
    pub post: fn(&'static RefCode, &str),
    /// The host's identity, which every pool disk of its carries.
    pub owner: fn() -> Option<[u8; 16]>,
    /// The run directory: the socket and the devices' numbers.
    pub run_dir: &'static str,
    /// Where pools are mounted.
    pub pools_dir: &'static str,
    /// The state file the labels are written out to.
    pub state_file: &'static str,
    /// The profiles of partitions that name a virtual disk, by its pool
    /// and its name: each as `PROD profile NORMAL slot 2`.
    pub named: fn(&str, &str) -> Vec<String>,
}

/// How often the loop looks, with nothing to do.
const TURN: Duration = Duration::from_millis(100);
/// Megabytes a pool's work copies between two looks at the socket.
const STEP_CHUNKS: u64 = 16;
/// How long work stands back while partitions' transfers are under way.
const GIVE_WAY: Duration = Duration::from_millis(20);
/// How often a pool's fill is looked at.
const FILL_EVERY: Duration = Duration::from_secs(5);

fn clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The service's state.
pub struct Core {
    host: Host,
    owner: [u8; 16],
    pools: Vec<Pool>,
    /// Who holds what.
    holders: Vec<proto::Held>,
    /// An import from a device of the host, and how far it is.
    import: Option<(String, u64, u64, String)>,
    stopped: bool,
    filled: Instant,
}

/// The service, shared by the loop and the socket's connections.
pub type Shared = Arc<Mutex<Core>>;

fn lock(core: &Shared) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The size of `disk` and what the inventory says of it.
fn disk_facts(inventory: &[DiskIdentity], disk: &str) -> (u64, String, String) {
    let id = inventory.iter().find(|d| d.name == disk);
    (
        disks::disk_bytes(disk),
        id.map(|d| d.serial.clone()).unwrap_or_default(),
        id.map(|d| d.location.clone()).unwrap_or_default(),
    )
}

/// A disk's logical block size.
fn logical_block(disk: &str) -> u32 {
    fs::read_to_string(format!("/sys/class/block/{disk}/queue/logical_block_size"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(512)
}

impl Core {
    fn post(&self, code: &'static RefCode, detail: &str) {
        (self.host.post)(code, detail);
    }

    fn names(&self) -> &'static PoolDiskNames {
        self.host.names
    }

    fn id_file(&self, label: &PoolLabel) -> PathBuf {
        Path::new(self.host.run_dir)
            .join("poold")
            .join(format!("{}.id", label.pool))
    }

    fn pools_dir(&self) -> &Path {
        Path::new(self.host.pools_dir)
    }

    /// Every whole disk and what it is.
    fn scan(&self) -> Vec<(String, Kind)> {
        let load_sources: Vec<String> = disks::load_sources(self.host.load_source)
            .into_iter()
            .map(|s| s.disk)
            .collect();
        disks::whole_disks()
            .into_iter()
            .map(|d| {
                let kind = if load_sources.contains(&d) {
                    Kind::LoadSource
                } else {
                    disk::classify(self.names(), &Path::new("/dev").join(&d), self.owner)
                };
                (d, kind)
            })
            .collect()
    }

    /// Find the pools by their disks' labels and assemble each.
    fn assemble(&mut self) {
        let scanned = self.scan();
        let mut labels = Vec::new();
        let mut found: Vec<(Found, Uuid)> = Vec::new();
        for (d, kind) in &scanned {
            if let Kind::Pool { label, partition } = kind {
                let record = disks::partition_node(d, 1);
                let member = disks::partition_node(d, 2);
                if let (Some(record), Some(member)) = (record, member) {
                    found.push((
                        Found {
                            disk: d.clone(),
                            record,
                            member,
                            partition: *partition,
                        },
                        label.pool,
                    ));
                }
                labels.push(label.clone());
            }
        }
        for l in label::newest(&labels) {
            let mine: Vec<Found> = found
                .iter()
                .filter(|(f, p)| *p == l.pool && l.unit_of(f.partition).is_some())
                .map(|(f, _)| f.clone())
                .collect();
            let mut p = Pool::new(l, mine);
            self.bring_up(&mut p);
            self.pools.push(p);
        }
        self.write_state();
    }

    /// Open, serve and mount one pool found at the start.
    fn bring_up(&self, p: &mut Pool) {
        let name = p.label.name.clone();
        // A unit not found is missing from now on.
        let present: Vec<Uuid> = p.found.iter().map(|f| f.partition).collect();
        for u in &mut p.label.units {
            if !present.contains(&u.partition) && u.state != UnitState::Spare {
                u.state = UnitState::Missing;
            }
        }
        for u in p.label.members().filter(|u| u.state == UnitState::Missing) {
            self.post(
                &refcode::POOL_UNIT_FAILED,
                &format!(
                    "{name}, {} serial {}: missing",
                    where_(&u.location),
                    u.serial
                ),
            );
        }
        if p.label.lost() {
            p.failure = format!(
                "{} unit(s) out of step; {} protection covers {}",
                p.label.out_of_step(),
                p.label.protection.word(),
                p.label.protection.tolerates()
            );
            self.post(&refcode::POOL_FAILED, &format!("{name}: {}", p.failure));
            return;
        }
        let at = create::mount_point(self.pools_dir(), &name);
        let id_file = self.id_file(&p.label);
        let up = p
            .open(self.names(), false)
            .and_then(|()| p.serve(&id_file))
            .and_then(|node| p.mount_at(&node, &at));
        if let Err(e) = up {
            p.failure = e;
            self.post(
                &refcode::POOL_NOT_ASSEMBLED,
                &format!("{name}: {}", p.failure),
            );
            return;
        }
        p.label.clean = false;
        p.write_label(self.names(), clock(), true);
        self.post(
            &refcode::POOL_ASSEMBLED,
            &format!("{name}, {}, {}", p.label.protection.word(), p.state()),
        );
        if let Some(Set::Mirror(m)) = &p.set
            && let Some(r) = m.resync_progress()
        {
            self.post(
                &refcode::POOL_RESYNC_STARTED,
                &format!("{name}, {} of {} MiB to copy", r.chunks_dirty, r.chunks),
            );
        }
        if let Some(Set::Parity(s)) = &p.set
            && let Some(ParityWork::Sync(done)) = s.work()
        {
            self.post(
                &refcode::POOL_RESYNC_STARTED,
                &format!("{name}, parity from block {done}"),
            );
        }
        let _ = library::Library::of(p).prepare();
    }

    /// The labels, written out: what the host's pools are. Made again
    /// from the labels at every start.
    fn write_state(&self) {
        let mut text = String::from("schema_version = 1\n");
        for p in &self.pools {
            let _ = write!(
                text,
                "\n[[pool]]\nname = \"{}\"\nuuid = \"{}\"\nprotection = \"{}\"\nunits = {}\n",
                p.label.name,
                p.label.pool,
                p.label.protection.word(),
                p.label.units.len()
            );
        }
        let _ = state::store::atomic_write(Path::new(self.host.state_file), text.as_bytes());
    }

    fn inventory(&self) -> Vec<DiskIdentity> {
        (self.host.inventory)()
    }

    /// Every disk and what it is, for Display disk configuration.
    fn disk_list(&self) -> Vec<Disk> {
        let inventory = self.inventory();
        self.scan()
            .into_iter()
            .map(|(name, kind)| {
                let (bytes, serial, location) = disk_facts(&inventory, &name);
                let pool = match &kind {
                    Kind::Pool { label, .. } => label.name.clone(),
                    _ => String::new(),
                };
                Disk {
                    logical_block: logical_block(&name),
                    what: kind.word(),
                    free: kind.free(),
                    foreign: matches!(kind, Kind::Foreign(_) | Kind::ForeignPool { .. }),
                    name,
                    serial,
                    location,
                    bytes,
                    pool,
                }
            })
            .collect()
    }

    fn pool_index(&self, name: &str) -> Option<usize> {
        self.pools.iter().position(|p| p.label.name == name)
    }

    /// Every pool as listed.
    fn pool_list(&self) -> Vec<PoolStatus> {
        let inventory = self.inventory();
        let facts = |d: &str| disk_facts(&inventory, d);
        let repository = self.repository();
        self.pools
            .iter()
            .map(|p| {
                let mut s = p.status(&facts, repository.as_deref() == Some(&p.label.name));
                s.provisioned = library::provisioned(p);
                s
            })
            .collect()
    }

    /// The pool that holds the media library.
    fn repository(&self) -> Option<String> {
        self.pools
            .iter()
            .find(|p| library::Library::of(p).designated())
            .map(|p| p.label.name.clone())
    }

    /// One turn of the loop for pool `i`.
    fn turn(&mut self, i: usize) {
        let names = self.names();
        let Some(set) = self.pools[i].set.clone() else {
            return;
        };
        let name = self.pools[i].label.name.clone();
        for event in set.events() {
            if let SetEvent::Failed { member, what } = event {
                let u = self.pools[i].label.units.get(member).cloned();
                let (serial, location) = u.map(|u| (u.serial, u.location)).unwrap_or_default();
                self.post(
                    &refcode::POOL_UNIT_FAILED,
                    &format!(
                        "{name}, unit {} {} serial {serial}: {what}",
                        member + 1,
                        where_(&location)
                    ),
                );
                if self.pools[i]
                    .work
                    .as_ref()
                    .is_some_and(|w| w.member == member)
                {
                    self.pools[i].work = None;
                }
            }
        }
        self.pools[i].write_label(names, clock(), false);
        // A mirror keeps its last unit in service whatever it does: a
        // transfer the set failed says no unit was left to serve it.
        let lost = self.pools[i].label.lost() || self.pools[i].recorded.failed();
        if lost && self.pools[i].failure.is_empty() {
            let p = &mut self.pools[i];
            p.recorded.lose();
            p.failure = if p.label.lost() {
                format!(
                    "{} unit(s) out of step; {} protection covers {}",
                    p.label.out_of_step(),
                    p.label.protection.word(),
                    p.label.protection.tolerates()
                )
            } else {
                "a transfer failed on every unit that holds it".into()
            };
            let detail = format!("{name}: {}", p.failure);
            self.post(&refcode::POOL_FAILED, &detail);
            return;
        }
        if !self.pools[i].failure.is_empty() {
            return;
        }
        self.take_spare(i, &set);
        self.work(i, &set);
    }

    /// A failed member's place given to a hot spare.
    fn take_spare(&mut self, i: usize, set: &Set) {
        let p = &mut self.pools[i];
        if p.spares.is_empty() {
            return;
        }
        let taken = match set {
            Set::Mirror(m) => m.take_spare(&mut p.spares),
            Set::Parity(s) => s.take_spare(&mut p.spares),
            Set::Plain(_) => None,
        };
        let Some((member, uuid)) = taken else { return };
        let spare = p
            .label
            .units
            .iter()
            .find(|u| *u.partition.as_bytes() == uuid)
            .cloned();
        let old = p.label.units.get(member).cloned();
        if let (Some(spare), Some(unit)) = (spare, p.label.units.get_mut(member)) {
            unit.serial.clone_from(&spare.serial);
            unit.location.clone_from(&spare.location);
        }
        let names = self.host.names;
        p.write_label(names, clock(), true);
        let detail = format!(
            "{}, unit {}: {} in place of {}",
            p.label.name,
            member + 1,
            where_(
                &p.label
                    .units
                    .get(member)
                    .map(|u| u.location.clone())
                    .unwrap_or_default()
            ),
            where_(&old.map(|u| u.location).unwrap_or_default()),
        );
        let started = create::start_rebuild(names, p, member);
        self.post(&refcode::POOL_SPARE_TAKEN, &detail);
        match started {
            Ok(()) => self.post(&refcode::POOL_REBUILD_STARTED, &detail),
            Err(e) => self.post(&refcode::POOL_REBUILD_FAILED, &format!("{detail}: {e}")),
        }
    }

    /// A step of the pool's work.
    #[allow(clippy::too_many_lines)]
    fn work(&mut self, i: usize, set: &Set) {
        let names = self.names();
        let name = self.pools[i].label.name.clone();
        let busy = {
            let r = self.pools[i].recorded.clone();
            move || r.busy()
        };
        let give_way = || {
            let until = Instant::now() + GIVE_WAY;
            while busy() && Instant::now() < until {
                std::thread::sleep(Duration::from_micros(200));
            }
        };
        match set {
            Set::Mirror(m) => {
                if let Some(mut w) = self.pools[i].work.take() {
                    let mut journal = runtime::journal(names, &self.pools[i]);
                    let mut step = m.resume_step(&mut w.resume, &mut journal, 1);
                    for _ in 1..STEP_CHUNKS {
                        if !matches!(&step, Ok(p) if !p.complete) {
                            break;
                        }
                        give_way();
                        step = m.resume_step(&mut w.resume, &mut journal, 1);
                    }
                    match step {
                        Ok(p) if p.complete => {
                            self.pools[i].write_label(names, clock(), true);
                            self.post(
                                &refcode::POOL_REBUILD_FINISHED,
                                &format!("{name}, unit {}, {} MiB", w.member + 1, p.chunks),
                            );
                        }
                        Ok(_) => self.pools[i].work = Some(w),
                        Err(e) => self.post(
                            &refcode::POOL_REBUILD_FAILED,
                            &format!("{name}, unit {}: {e}", w.member + 1),
                        ),
                    }
                    return;
                }
                if m.resync_progress().is_some_and(|p| !p.complete) {
                    let mut step = m.resync_step(1);
                    for _ in 1..STEP_CHUNKS {
                        if !matches!(&step, Ok(Some(p)) if !p.complete) {
                            break;
                        }
                        give_way();
                        step = m.resync_step(1);
                    }
                    match step {
                        Ok(Some(p)) if p.complete => {
                            self.pools[i].write_label(names, clock(), true);
                            self.post(
                                &refcode::POOL_RESYNC_FINISHED,
                                &format!("{name}, {} of {} MiB copied", p.chunks_copied, p.chunks),
                            );
                        }
                        Ok(_) => {}
                        Err(e) => self.post(&refcode::POOL_REBUILD_FAILED, &format!("{name}: {e}")),
                    }
                    return;
                }
                // A unit that is there and stale is brought back.
                let stale = m.members().iter().position(|x| {
                    x.attached
                        && matches!(
                            x.state,
                            protect::UnitState::Suspended | protect::UnitState::Resuming
                        )
                });
                if let Some(member) = stale
                    && !self.pools[i].said.contains(&format!("stale {member}"))
                {
                    self.pools[i].said.push(format!("stale {member}"));
                    match create::start_rebuild(names, &mut self.pools[i], member) {
                        Ok(()) => self.post(
                            &refcode::POOL_REBUILD_STARTED,
                            &format!("{name}, unit {}", member + 1),
                        ),
                        Err(e) => self.post(
                            &refcode::POOL_REBUILD_FAILED,
                            &format!("{name}, unit {}: {e}", member + 1),
                        ),
                    }
                }
            }
            Set::Parity(s) => {
                let Some(work) = s.work() else { return };
                let key = match work {
                    ParityWork::Sync(_) => "sync".to_string(),
                    ParityWork::Rebuild(member, _) => format!("rebuild {member}"),
                };
                if let ParityWork::Rebuild(member, _) = work
                    && !self.pools[i].said.contains(&key)
                {
                    self.post(
                        &refcode::POOL_REBUILD_STARTED,
                        &format!("{name}, unit {}", member + 1),
                    );
                }
                if !self.pools[i].said.contains(&key) {
                    self.pools[i].said.push(key.clone());
                }
                give_way();
                match s.step(256 * STEP_CHUNKS) {
                    Ok(Some(p)) if p.complete => {
                        self.pools[i].said.retain(|k| *k != key);
                        self.pools[i].write_label(names, clock(), true);
                        let code = match p.work {
                            ParityWork::Sync(_) => &refcode::POOL_RESYNC_FINISHED,
                            ParityWork::Rebuild(..) => &refcode::POOL_REBUILD_FINISHED,
                        };
                        self.post(code, &format!("{name}, {} blocks", p.blocks));
                    }
                    Ok(_) => self.pools[i].write_label(names, clock(), false),
                    Err(e) => self.post(&refcode::POOL_REBUILD_FAILED, &format!("{name}: {e}")),
                }
            }
            Set::Plain(_) => {}
        }
    }

    /// Look at each pool's fill, and warn at 80% and 90%.
    fn fill(&mut self) {
        if self.filled.elapsed() < FILL_EVERY {
            return;
        }
        self.filled = Instant::now();
        let mut posts = Vec::new();
        for p in &mut self.pools {
            let Some(at) = &p.mount else { continue };
            let (size, used) = runtime::space(at);
            if size == 0 {
                continue;
            }
            let percent = used * 100 / size;
            let level = if percent >= 90 {
                90
            } else if percent >= 80 {
                80
            } else {
                0
            };
            if level > p.warned {
                let code = if level == 90 {
                    &refcode::POOL_NEARLY_FULL
                } else {
                    &refcode::POOL_FILLING
                };
                posts.push((
                    code,
                    format!(
                        "{}, {used} of {size} bytes allocated, {} provisioned",
                        p.label.name,
                        library::provisioned(p)
                    ),
                ));
            }
            if level > p.warned || percent < 75 {
                p.warned = level;
            }
        }
        for (code, detail) in posts {
            self.post(code, &detail);
        }
    }

    /// Stop every pool in order. How many were stopped.
    fn stop_all(&mut self) -> u32 {
        if self.stopped {
            return 0;
        }
        let names = self.names();
        let mut n = 0;
        for p in &mut self.pools {
            p.stop(names, clock());
            n += 1;
        }
        self.stopped = true;
        self.post(&refcode::POOLS_STOPPED, &format!("{n} pool(s)"));
        n
    }
}

use uuid::Uuid;

/// `at <location>` when there is one.
fn where_(location: &str) -> String {
    if location.is_empty() {
        "at an unknown location".into()
    } else {
        format!("at {location}")
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

fn serve_socket(listener: &UnixListener, core: &Shared) {
    for stream in listener.incoming().flatten() {
        let core = core.clone();
        let _ = std::thread::Builder::new()
            .name("connection".into())
            .spawn(move || library::connection(&core, &stream));
    }
}

/// Run the pool service for `host`. It returns only when it could not
/// start.
pub fn run(host: Host) -> String {
    let mut signals = match Signals::take(&[TERM]) {
        Ok(signals) => signals,
        Err(e) => return format!("the signal descriptor: {e}"),
    };
    let Some(owner) = (host.owner)() else {
        return "the host has no identity yet".into();
    };
    let _ = fs::create_dir_all(Path::new(host.run_dir).join("poold"));
    let _ = fs::create_dir_all(host.pools_dir);
    let socket = proto::socket(host.run_dir);
    let _ = fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => return format!("{}: {e}", socket.display()),
    };
    // The QSI host server asks too.
    if let Err(e) = release::share_with_server(&socket) {
        return format!("{}: {e}", socket.display());
    }
    let mut core = Core {
        host,
        owner,
        pools: Vec::new(),
        holders: library::read_holders(host.run_dir),
        import: None,
        stopped: false,
        filled: Instant::now(),
    };
    core.assemble();
    let assembled = core.pools.iter().filter(|p| p.mount.is_some()).count();
    core.post(
        &refcode::POOLD_STARTED,
        &format!("{} pool(s) found, {assembled} assembled", core.pools.len()),
    );
    let core: Shared = Arc::new(Mutex::new(core));
    let for_socket = core.clone();
    let _ = std::thread::Builder::new()
        .name("socket".into())
        .spawn(move || serve_socket(&listener, &for_socket));
    if let Err(e) = supervise::ready::signal() {
        return format!("the readiness pipe: {e}");
    }
    loop {
        let busy = lock(&core).pools.iter().any(Pool::busy);
        let wait = if busy { Duration::ZERO } else { TURN };
        if term(&mut signals, wait) {
            lock(&core).stop_all();
            std::process::exit(0);
        }
        let mut c = lock(&core);
        for i in 0..c.pools.len() {
            c.turn(i);
            if let Some(Set::Mirror(m)) = &c.pools[i].set {
                let _ = m.sweep();
            }
        }
        c.fill();
    }
}

/// A connection's stream, for the library's requests.
pub type Stream = UnixStream;
