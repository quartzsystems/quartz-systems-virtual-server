// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One pool at run time: its set over the units that are there, served
//! to the kernel through ublk, its file system mounted, its label kept
//! in step with its units, and the work a unit's failure or an unclean
//! stop leaves: a rebuild, a resynchronization, a parity computed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use blockdev::{BlockDev, LinuxBlockDev};
use pool::label::{PoolLabel, Protection, UnitState};
use pool::layout::{AWAY_MAP_OFFSET, SET_MAP_OFFSET};
use pool::names::PoolDiskNames;
use protect::UnitState as Mirror;
use protect::intent::IntentStore;
use protect::parity::{Format, Geometry, Health, Level};
use protect::set::{
    Intent, MirrorMember, MirrorSet, ParityMember, ParitySet, ParityWork, Resume, SetEvent,
};
use protect::spare::Spare;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use ublk::{BlockTarget, Device, Options, Target};
use uuid::Uuid;

use super::guard::{Guarded, Recorded};
use super::records::{self, Journal};
use crate::proto::{PoolStatus, PoolUnit, state};

/// A disk of the pool that is there.
#[derive(Clone, Debug)]
pub struct Found {
    /// Its kernel name.
    pub disk: String,
    /// Its record partition's node.
    pub record: PathBuf,
    /// Its member partition's node.
    pub member: PathBuf,
    /// Its member partition's GUID: the unit's identity.
    pub partition: Uuid,
}

/// The set a pool presents.
#[derive(Clone)]
pub enum Set {
    /// One disk.
    Plain(Arc<LinuxBlockDev>),
    /// Two, mirrored.
    Mirror(MirrorSet),
    /// Three or more, with parity.
    Parity(ParitySet),
}

/// Work a mirrored pool has under way.
pub struct Work {
    /// The member being copied to.
    pub member: usize,
    /// Its resume.
    pub resume: Resume,
    /// A rebuild of a new unit, or a stale one brought back.
    pub rebuild: bool,
}

/// One pool.
pub struct Pool {
    /// What its disks say of it.
    pub label: PoolLabel,
    /// Its disks that are there.
    pub found: Vec<Found>,
    /// Its set, once opened.
    pub set: Option<Set>,
    /// The ublk device serving the set.
    pub device: Option<Device>,
    /// Where its file system is mounted.
    pub mount: Option<PathBuf>,
    /// Why it is Failed or was not assembled.
    pub failure: String,
    /// A mirrored pool's resume under way.
    pub work: Option<Work>,
    /// The states the label on the disks has.
    pub recorded: Arc<Recorded>,
    /// Its hot spares that are there.
    pub spares: Vec<Spare>,
    /// The highest allocation warning given: 0, 80 or 90.
    pub warned: u8,
    /// A unit failed or a hot spare was taken: said once.
    pub said: Vec<String>,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The parity set's own records' magics.
pub fn format(names: &PoolDiskNames) -> Format {
    Format {
        ppl_magic: names.ppl_magic,
        state_magic: names.parity_state_magic,
        suspicious: |_| false,
    }
}

fn level(p: Protection) -> Level {
    if p == Protection::Raid6 {
        Level::Raid6
    } else {
        Level::Raid5
    }
}

fn mirror_state(s: UnitState) -> Mirror {
    match s {
        UnitState::Active => Mirror::Active,
        UnitState::Suspended => Mirror::Suspended,
        UnitState::Resuming | UnitState::Rebuilding(_) => Mirror::Resuming,
        UnitState::Failed => Mirror::NotOperational,
        UnitState::Missing | UnitState::Spare => Mirror::Missing,
    }
}

fn from_mirror(s: Mirror) -> UnitState {
    match s {
        Mirror::Active | Mirror::Unprotected => UnitState::Active,
        Mirror::Suspended => UnitState::Suspended,
        Mirror::Resuming => UnitState::Resuming,
        Mirror::NotOperational => UnitState::Failed,
        Mirror::Missing => UnitState::Missing,
    }
}

fn health(s: UnitState) -> Health {
    match s {
        UnitState::Active => Health::Ok,
        UnitState::Rebuilding(mark) => Health::Rebuilding(mark),
        _ => Health::Failed,
    }
}

fn from_health(h: Health, attached: bool) -> UnitState {
    match h {
        Health::Ok => UnitState::Active,
        Health::Rebuilding(mark) => UnitState::Rebuilding(mark),
        Health::Failed if attached => UnitState::Failed,
        Health::Failed => UnitState::Missing,
    }
}

impl Set {
    /// The units' states as one number, for the guard.
    pub fn states(&self) -> u64 {
        match self {
            Set::Plain(_) => 0,
            Set::Mirror(m) => m
                .members()
                .iter()
                .fold(0, |all, x| all << 8 | u64::from(x.state.as_u8())),
            Set::Parity(p) => p.members().iter().fold(0, |all, x| {
                all << 8
                    | match x.health {
                        Health::Ok => 1,
                        Health::Rebuilding(_) => 2,
                        Health::Failed => 3,
                    }
            }),
        }
    }

    /// What the set did by itself since the last look.
    pub fn events(&self) -> Vec<SetEvent> {
        match self {
            Set::Plain(_) => Vec::new(),
            Set::Mirror(m) => m.drain_events(),
            Set::Parity(p) => p.drain_events(),
        }
    }

    fn target(&self) -> Box<dyn Target> {
        match self {
            Set::Plain(d) => Box::new(BlockTarget(d.clone())),
            Set::Mirror(m) => Box::new(BlockTarget(m.clone())),
            Set::Parity(p) => Box::new(BlockTarget(p.clone())),
        }
    }
}

impl Pool {
    /// A pool known by `label`, its disks `found`, not yet opened.
    pub fn new(label: PoolLabel, found: Vec<Found>) -> Self {
        Self {
            label,
            found,
            set: None,
            device: None,
            mount: None,
            failure: String::new(),
            work: None,
            recorded: Arc::default(),
            spares: Vec::new(),
            warned: 0,
            said: Vec::new(),
        }
    }

    fn found_for(&self, partition: Uuid) -> Option<&Found> {
        self.found.iter().find(|f| f.partition == partition)
    }

    /// The record partitions of the disks that are there.
    pub fn records(&self) -> Vec<PathBuf> {
        self.found.iter().map(|f| f.record.clone()).collect()
    }

    fn open_member(&self, partition: Uuid) -> Option<Box<dyn BlockDev>> {
        let f = self.found_for(partition)?;
        LinuxBlockDev::open(&f.member)
            .ok()
            .map(|d| Box::new(d) as Box<dyn BlockDev>)
    }

    /// Open the set over the units that are there; `fresh` for a set
    /// just made, whose members hold nothing yet.
    pub fn open(&mut self, names: &'static PoolDiskNames, fresh: bool) -> Result<(), String> {
        let l = self.label.clone();
        let id = *l.pool.as_bytes();
        let members: Vec<_> = l.members().cloned().collect();
        let set = match l.protection {
            Protection::None => {
                let m = members.first().ok_or("the pool has no unit")?;
                let f = self.found_for(m.partition).ok_or("its unit is not there")?;
                Set::Plain(Arc::new(LinuxBlockDev::open(&f.member).map_err(text)?))
            }
            Protection::Mirrored => {
                let records = self.records();
                let mut intent = Intent::new(
                    names.intent_magic,
                    Box::new(records::intent_map(names, &records, SET_MAP_OFFSET)),
                );
                intent.region_blocks = loadsource::intent::region_blocks(l.set_blocks);
                intent.fresh = fresh || l.clean;
                intent.away = members
                    .iter()
                    .map(|m| {
                        let others: Vec<PathBuf> = self
                            .found
                            .iter()
                            .filter(|f| f.partition != m.partition)
                            .map(|f| f.record.clone())
                            .collect();
                        let store = records::intent_map(names, &others, AWAY_MAP_OFFSET);
                        Some(Box::new(store) as Box<dyn IntentStore>)
                    })
                    .collect();
                let units = members
                    .iter()
                    .map(|m| MirrorMember {
                        uuid: *m.partition.as_bytes(),
                        state: mirror_state(m.state),
                        dev: self.open_member(m.partition),
                    })
                    .collect();
                Set::Mirror(
                    MirrorSet::open_with(id, l.set_blocks, units, Some(intent)).map_err(text)?,
                )
            }
            Protection::Raid5 | Protection::Raid6 => {
                let geo = Geometry {
                    level: level(l.protection),
                    n: members.len(),
                    blocks: l.parity.0,
                    head: l.parity.1,
                };
                let units = members
                    .iter()
                    .map(|m| ParityMember {
                        uuid: *m.partition.as_bytes(),
                        health: health(m.state),
                        dev: self.open_member(m.partition),
                    })
                    .collect();
                Set::Parity(ParitySet::open(format(names), id, geo, units, l.sync).map_err(text)?)
            }
        };
        self.spares = l
            .spares()
            .filter_map(|s| {
                Some(Spare {
                    uuid: *s.partition.as_bytes(),
                    dev: self.open_member(s.partition)?,
                })
            })
            .collect();
        self.recorded.set(set.states());
        self.set = Some(set);
        Ok(())
    }

    /// Serve the set to the kernel: a device this boot already made is
    /// taken over (the service was restarted), else one is made.
    pub fn serve(&mut self, id_file: &Path) -> Result<PathBuf, String> {
        let set = self.set.clone().ok_or("the set is not open")?;
        let kept = fs::read_to_string(id_file)
            .ok()
            .and_then(|id| id.trim().parse::<u32>().ok())
            .filter(|id| Path::new(&format!("/dev/ublkb{id}")).exists());
        let target = Arc::new(Guarded::new(
            set.target(),
            Arc::new(move || set.states()),
            Arc::clone(&self.recorded),
        ));
        let device = match kept {
            Some(id) => Device::recover(id, target),
            None => Device::start_with(target, Options { recovery: true }),
        }
        .map_err(text)?;
        let _ = fs::write(id_file, device.id().to_string());
        let node = device.block_path();
        let give_up = Instant::now() + Duration::from_secs(10);
        while !node.exists() {
            if Instant::now() > give_up {
                return Err(format!("{} did not appear", node.display()));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.device = Some(device);
        Ok(node)
    }

    /// Mount the pool's file system at `at`, unless it is mounted there.
    pub fn mount_at(&mut self, node: &Path, at: &Path) -> Result<(), String> {
        fs::create_dir_all(at).map_err(text)?;
        let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
        let shown = at.display().to_string();
        let already = mounts
            .lines()
            .any(|l| l.split(' ').nth(1) == Some(shown.as_str()));
        if !already {
            let flags = MountFlags::NODEV | MountFlags::NOSUID | MountFlags::NOEXEC;
            mount(node, at, "xfs", flags, None).map_err(|e| format!("{}: {e}", at.display()))?;
        }
        self.mount = Some(at.to_path_buf());
        Ok(())
    }

    /// Bring the label in step with the set's units and write it to
    /// every disk of the pool that is there, when it changed or `force`.
    pub fn write_label(&mut self, names: &'static PoolDiskNames, now: u64, force: bool) {
        let Some(set) = &self.set else { return };
        let mut label = self.label.clone();
        let members: Vec<Uuid> = label.members().map(|u| u.partition).collect();
        match set {
            Set::Plain(_) => {}
            Set::Mirror(m) => {
                for (i, now) in m.members().iter().enumerate() {
                    if let Some(u) = label.units.get_mut(i) {
                        u.partition = Uuid::from_bytes(now.uuid);
                        u.state = if now.attached || now.state != Mirror::Missing {
                            from_mirror(now.state)
                        } else {
                            UnitState::Missing
                        };
                    }
                }
            }
            Set::Parity(p) => {
                for (i, now) in p.members().iter().enumerate() {
                    if let Some(u) = label.units.get_mut(i) {
                        u.partition = Uuid::from_bytes(now.uuid);
                        u.state = from_health(now.health, now.attached);
                    }
                }
                label.sync = p.sync();
            }
        }
        // A spare taken is a member now: it leaves the spares' list.
        let taken: Vec<Uuid> = label
            .units
            .iter()
            .take(members.len())
            .map(|u| u.partition)
            .collect();
        let n = members.len();
        label.units = label
            .units
            .iter()
            .enumerate()
            .filter(|(i, u)| *i < n || !taken.contains(&u.partition))
            .map(|(_, u)| u.clone())
            .collect();
        if !force && label == self.label {
            self.recorded.set(set.states());
            return;
        }
        label.generation += 1;
        label.written = now;
        if !records::write_label(names, &self.records(), &label) {
            eprintln!("pool {}: the label was not written", label.name);
        }
        self.recorded.set(set.states());
        self.label = label;
    }

    /// The pool's state.
    pub fn state(&self) -> &'static str {
        if !self.failure.is_empty() && self.set.is_none() {
            return state::FAILED;
        }
        if self.label.lost() || self.recorded.failed() {
            return state::FAILED;
        }
        if self.set.is_none() {
            return state::ASSEMBLING;
        }
        let rebuilding = self
            .label
            .members()
            .any(|u| matches!(u.state, UnitState::Rebuilding(_) | UnitState::Resuming));
        if self.work.as_ref().is_some_and(|w| w.rebuild) || rebuilding {
            return state::REBUILDING;
        }
        if self.resyncing() {
            return state::RESYNCHRONIZING;
        }
        if self.label.out_of_step() > 0 {
            return state::DEGRADED;
        }
        state::ACTIVE
    }

    /// Whether the units are being brought in step without a unit
    /// out of service: after an unclean stop, or a new parity set.
    pub fn resyncing(&self) -> bool {
        match &self.set {
            Some(Set::Mirror(m)) => m.resync_progress().is_some_and(|p| !p.complete),
            Some(Set::Parity(p)) => matches!(p.work(), Some(ParityWork::Sync(_))),
            _ => false,
        }
    }

    /// Work for the loop: whether there is any.
    pub fn busy(&self) -> bool {
        self.work.is_some()
            || self.resyncing()
            || matches!(&self.set, Some(Set::Parity(p)) if p.work().is_some())
    }

    /// The work under way, and how far: for the status.
    fn progress(&self) -> (String, u32, u32) {
        if let Some(w) = &self.work {
            let what = if w.rebuild {
                "rebuilding"
            } else {
                "resynchronizing"
            };
            let p = w.resume.progress();
            return (
                what.into(),
                u32::try_from(w.member + 1).unwrap_or(0),
                u32::from(p.percent),
            );
        }
        match &self.set {
            Some(Set::Mirror(m)) => match m.resync_progress() {
                Some(p) if !p.complete => (
                    "resynchronizing".into(),
                    0,
                    u32::try_from(p.chunks_copied * 100 / p.chunks_dirty.max(1)).unwrap_or(0),
                ),
                _ => (String::new(), 0, 0),
            },
            Some(Set::Parity(p)) => {
                let blocks = self.label.set_blocks.max(1);
                match p.work() {
                    Some(ParityWork::Sync(done)) => (
                        "resynchronizing".into(),
                        0,
                        u32::try_from(done * 100 / blocks).unwrap_or(0),
                    ),
                    Some(ParityWork::Rebuild(member, done)) => (
                        "rebuilding".into(),
                        u32::try_from(member + 1).unwrap_or(0),
                        u32::try_from(done * 100 / blocks).unwrap_or(0),
                    ),
                    None => (String::new(), 0, 0),
                }
            }
            _ => (String::new(), 0, 0),
        }
    }

    /// The pool as a list shows it.
    pub fn status(
        &self,
        disk_of: &dyn Fn(&str) -> (u64, String, String),
        repository: bool,
    ) -> PoolStatus {
        let (work, work_unit, percent) = self.progress();
        let (bytes, allocated) = self.mount.as_deref().map_or((0, 0), space);
        PoolStatus {
            name: self.label.name.clone(),
            uuid: self.label.pool.to_string(),
            protection: self.label.protection.word().into(),
            state: self.state().into(),
            work,
            work_unit,
            percent,
            bytes,
            allocated,
            provisioned: 0,
            mount: self
                .mount
                .as_ref()
                .map(|m| m.display().to_string())
                .unwrap_or_default(),
            repository,
            units: self
                .label
                .units
                .iter()
                .enumerate()
                .map(|(i, u)| {
                    let disk = self
                        .found_for(u.partition)
                        .map(|f| f.disk.clone())
                        .unwrap_or_default();
                    let (bytes, _, _) = disk_of(&disk);
                    PoolUnit {
                        number: u32::try_from(i + 1).unwrap_or(0),
                        disk,
                        serial: u.serial.clone(),
                        location: u.location.clone(),
                        bytes,
                        state: u.state.word().into(),
                    }
                })
                .collect(),
            failure: self.failure.clone(),
        }
    }

    /// Stop in order: the file system unmounted, the device stopped,
    /// the set flushed and its maps cleared, the label marked clean.
    pub fn stop(&mut self, names: &'static PoolDiskNames, now: u64) {
        if let Some(at) = self.mount.take()
            && let Err(e) = unmount(&at, UnmountFlags::empty())
        {
            eprintln!("pool {}: {}: {e}", self.label.name, at.display());
        }
        let stopped = self.device.take().map_or(Ok(()), Device::stop);
        let closed = match &self.set {
            Some(Set::Mirror(m)) => m.close().is_ok(),
            Some(Set::Parity(p)) => p.flush().is_ok(),
            Some(Set::Plain(d)) => d.flush().is_ok(),
            None => true,
        };
        if stopped.is_ok() && closed {
            self.label.clean = true;
            self.write_label(names, now, true);
        }
    }
}

/// A file system's size and what is allocated in it.
pub fn space(at: &Path) -> (u64, u64) {
    match rustix::fs::statvfs(at) {
        Ok(s) => {
            let block = s.f_frsize;
            let size = s.f_blocks * block;
            (size, size - s.f_bfree * block)
        }
        Err(_) => (0, 0),
    }
}

/// A mirrored pool's resume journal on its disks.
pub fn journal(names: &'static PoolDiskNames, pool: &Pool) -> Journal {
    Journal {
        names,
        records: pool.records(),
    }
}
