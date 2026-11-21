// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The socket's requests: pools, virtual disks, the media library, and
//! who holds what. A partition's backing stores and media are handed
//! to the partition manager as open files, and the holder recorded in
//! the run directory, so a service started again knows them.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use pool::disk::Kind;
use pool::label::{Protection, UnitState};
use pool::plan::{self, Candidate, Problem};
use qvsmsg::Issued;

use super::runtime::Pool;
use super::{Core, Shared, clock, create, lock};
use crate::proto::{
    self, Disk, Given, Held, Holder, Image, Kind as Want_, PoolStatus, Refused, Request, Vdisk,
    Volume, Want,
};

/// The file in the repository's library that designates it.
const DESIGNATED: &str = "REPOSITORY";
/// The holders' record in the run directory.
const HOLDERS: &str = "poold/holders";
/// Bytes copied at a time in an upload or an import.
const CHUNK: usize = 1 << 20;

fn one(m: Issued) -> Refused {
    vec![m]
}

/// A pool's media library.
pub struct Library {
    mount: Option<PathBuf>,
}

impl Library {
    /// The library of `pool`, wherever it is mounted.
    pub fn of(pool: &Pool) -> Self {
        Self {
            mount: pool.mount.clone(),
        }
    }

    fn inner(&self) -> Option<media::Library> {
        self.mount.as_deref().map(media::Library::new)
    }

    /// Whether the pool is the media repository.
    pub fn designated(&self) -> bool {
        self.mount
            .as_ref()
            .is_some_and(|m| m.join(media::DIR).join(DESIGNATED).exists())
    }

    /// Make the library's directories and clear what an import left.
    pub fn prepare(&self) -> io::Result<()> {
        match self.inner() {
            Some(l) if self.designated() => l.prepare(),
            _ => Ok(()),
        }
    }
}

/// Bytes a pool's virtual disks and media are, whatever they hold.
pub fn provisioned(pool: &Pool) -> u64 {
    let Some(at) = &pool.mount else { return 0 };
    let disks: u64 = vdisk::list(at).iter().map(|d| d.bytes).sum();
    let l = media::Library::new(at);
    let images: u64 = l.images().iter().map(|i| i.bytes).sum();
    let volumes: u64 = l.volumes().iter().map(|v| v.volume.capacity()).sum();
    disks + images + volumes
}

/// The holders recorded in `run_dir`.
pub fn read_holders(run_dir: &str) -> Vec<Held> {
    fs::read(Path::new(run_dir).join(HOLDERS))
        .ok()
        .and_then(|b| ipc::decode(&b).ok())
        .unwrap_or_default()
}

fn write_holders(core: &Core) {
    let path = Path::new(core.host.run_dir).join(HOLDERS);
    if let Err(e) = state::store::atomic_write(&path, &ipc::encode(&core.holders)) {
        eprintln!("{}: {e}", path.display());
    }
}

fn holder_of(core: &Core, kind: Want_, pool: &str, name: &str) -> Holder {
    core.holders
        .iter()
        .find(|h| h.want.kind == kind && h.want.pool == pool && h.want.name == name)
        .map(|h| Holder {
            partition: h.partition,
            slot: h.want.slot,
        })
        .unwrap_or_default()
}

/// What `e` says, for a message.
fn said(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Core {
    fn repository_library(&self) -> Result<media::Library, Refused> {
        self.pools
            .iter()
            .find(|p| Library::of(p).designated())
            .and_then(|p| p.mount.as_deref())
            .map(media::Library::new)
            .ok_or_else(|| one(qvsmsg::NO_REPOSITORY.with(&[])))
    }

    fn usable_pool(&self, name: &str) -> Result<&Pool, Refused> {
        let i = self
            .pool_index(name)
            .ok_or_else(|| one(qvsmsg::SPOOL_MISSING.with(&[name])))?;
        let p = &self.pools[i];
        if p.mount.is_none() || !p.failure.is_empty() {
            return Err(one(qvsmsg::SPOOL_NOT_ACTIVE.with(&[name, p.state()])));
        }
        Ok(p)
    }

    /// The candidates for a pool: each disk named, and why it cannot be
    /// taken when it cannot.
    fn candidates(&self, disks: &[String], confirmed: &[String]) -> (Vec<Candidate>, Refused) {
        let scanned = self.scan();
        let mut missing = Vec::new();
        let list = disks
            .iter()
            .filter_map(|d| {
                let Some((_, kind)) = scanned.iter().find(|(n, _)| n == d) else {
                    missing.push(qvsmsg::DISK_MISSING.with(&[d]));
                    return None;
                };
                let taken = match kind {
                    Kind::LoadSource => Some("it is the load source".to_string()),
                    Kind::Pool { label, .. } => {
                        Some(format!("it is a disk of storage pool {}", label.name))
                    }
                    Kind::Foreign(_) | Kind::ForeignPool { .. } if !confirmed.contains(d) => {
                        missing.push(qvsmsg::DISK_FOREIGN.with(&[d, &kind.word()]));
                        None
                    }
                    _ if create::mounted(d) => Some("it is in use".to_string()),
                    _ => None,
                };
                Some(Candidate {
                    disk: d.clone(),
                    bytes: loadsource::disks::disk_bytes(d),
                    logical_block: super::logical_block(d),
                    taken,
                })
            })
            .collect();
        (list, missing)
    }

    fn problem(p: &Problem) -> Issued {
        match p {
            Problem::Members {
                protection,
                range,
                chosen,
            } => qvsmsg::SPOOL_MEMBERS.with(&[
                protection.word(),
                &range.0.to_string(),
                &range.1.to_string(),
                &chosen.to_string(),
            ]),
            Problem::Twice(d) => qvsmsg::DISK_TAKEN.with(&[d, "it is chosen twice"]),
            Problem::Taken(d, why) => qvsmsg::DISK_TAKEN.with(&[d, why]),
            Problem::TooSmall(d, _) => qvsmsg::DISK_TOO_SMALL.with(&[d]),
            Problem::BlockSizes(all) => {
                let text: Vec<String> = all.iter().map(|(d, b)| format!("{d} {b}")).collect();
                qvsmsg::BLOCK_SIZES.with(&[&text.join(", ")])
            }
            Problem::SpareTooSmall(d) => qvsmsg::SPARE_TOO_SMALL.with(&[d]),
            Problem::TooManySpares(n) => qvsmsg::SPOOL_SPARES.with(&[
                &n.to_string(),
                &format!("a pool has {} hot spares at most", plan::MAX_SPARES),
            ]),
            Problem::SpareUnprotected => qvsmsg::SPOOL_SPARES.with(&[
                "a hot spare",
                "a pool with no protection has no use for one",
            ]),
            Problem::TooLarge(b) => {
                qvsmsg::SPOOL_TOO_LARGE.with(&[&b.to_string(), &plan::MAX_POOL_BYTES.to_string()])
            }
        }
    }

    fn create_pool(
        &mut self,
        name: &str,
        protection: &str,
        members: &[String],
        spares: &[String],
        confirm: &[String],
    ) -> Result<PoolStatus, Refused> {
        let mut refused = Vec::new();
        if !pool::label::valid_name(name) {
            refused.push(qvsmsg::SPOOL_NAME_INVALID.with(&[name]));
        }
        if self.pool_index(name).is_some() {
            refused.push(qvsmsg::SPOOL_EXISTS.with(&[name]));
        }
        let Some(protection) = Protection::from_word(protection) else {
            refused.push(qvsmsg::SPOOL_MEMBERS.with(&[protection, "1", "18", "0"]));
            return Err(refused);
        };
        let (m, mut r1) = self.candidates(members, confirm);
        let (s, r2) = self.candidates(spares, confirm);
        refused.append(&mut r1);
        refused.extend(r2);
        let planned = plan::plan(self.names(), protection, &m, &s);
        if let Err(problems) = &planned {
            refused.extend(problems.iter().map(Self::problem));
        }
        if !refused.is_empty() {
            return Err(refused);
        }
        let Ok(planned) = planned else {
            return Err(refused);
        };
        let inventory = self.inventory();
        let identity = |d: &str| {
            let (_, serial, location) = super::disk_facts(&inventory, d);
            (serial, location)
        };
        let at = create::mount_point(self.pools_dir(), name);
        let id_file = self.id_file_for(name);
        let made = create::create(
            self.names(),
            self.owner,
            name,
            &planned,
            &identity,
            &id_file,
            &at,
            clock(),
        )
        .map_err(|e| one(qvsmsg::SPOOL_NOT_MADE.with(&[name, &e])))?;
        // The device's number goes under the pool's identity.
        let _ = fs::rename(&id_file, self.id_file(&made.label));
        self.post(
            &refcode::POOL_CREATED,
            &format!(
                "{name}, {}, {} unit(s), {} bytes",
                protection.word(),
                planned.members.len(),
                planned.capacity()
            ),
        );
        self.pools.push(made);
        self.write_state();
        Ok(self
            .pool_list()
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_default())
    }

    fn id_file_for(&self, name: &str) -> PathBuf {
        Path::new(self.host.run_dir)
            .join("poold")
            .join(format!("new-{name}.id"))
    }

    fn held_in(&self, pool: &str) -> Option<u32> {
        let repository = self.repository();
        self.holders
            .iter()
            .find(|h| {
                h.want.pool == pool
                    || (h.want.kind != Want_::Vdisk && repository.as_deref() == Some(pool))
            })
            .map(|h| h.partition)
    }

    fn delete_pool(&mut self, name: &str) -> Result<(), Refused> {
        let i = self
            .pool_index(name)
            .ok_or_else(|| one(qvsmsg::SPOOL_MISSING.with(&[name])))?;
        let p = &self.pools[i];
        if let Some(at) = &p.mount {
            let disks = vdisk::list(at).len();
            let l = media::Library::new(at);
            let media = l.images().len() + l.volumes().len();
            if disks + media > 0 {
                return Err(one(qvsmsg::SPOOL_NOT_EMPTY.with(&[
                    name,
                    &disks.to_string(),
                    &media.to_string(),
                ])));
            }
        }
        let mut p = self.pools.remove(i);
        p.stop(self.names(), clock());
        for f in &p.found {
            create::release_disk(f);
        }
        let _ = fs::remove_file(self.id_file(&p.label));
        if let Some(at) = &p.mount {
            let _ = fs::remove_dir(at);
        }
        let _ = fs::remove_dir(create::mount_point(self.pools_dir(), name));
        self.write_state();
        Ok(())
    }

    fn rename_pool(&mut self, name: &str, to: &str) -> Result<(), Refused> {
        if !pool::label::valid_name(to) {
            return Err(one(qvsmsg::SPOOL_NAME_INVALID.with(&[to])));
        }
        if self.pool_index(to).is_some() {
            return Err(one(qvsmsg::SPOOL_EXISTS.with(&[to])));
        }
        let i = self
            .pool_index(name)
            .ok_or_else(|| one(qvsmsg::SPOOL_MISSING.with(&[name])))?;
        if let Some(id) = self.held_in(name) {
            return Err(one(qvsmsg::SPOOL_IN_USE.with(&[name, &id.to_string()])));
        }
        let new_at = create::mount_point(self.pools_dir(), to);
        if let Some(at) = self.pools[i].mount.clone() {
            fs::create_dir_all(&new_at).map_err(|e| one(storage("rename", &e)))?;
            rustix::mount::mount_move(&at, &new_at).map_err(|e| one(storage("rename", &e)))?;
            let _ = fs::remove_dir(&at);
            self.pools[i].mount = Some(new_at);
        }
        let names = self.names();
        self.pools[i].label.name = to.to_string();
        self.pools[i].write_label(names, clock(), true);
        self.write_state();
        Ok(())
    }

    fn take_disk(&self, disk: &str, confirm: bool) -> Result<(), Refused> {
        let confirmed = if confirm {
            vec![disk.to_string()]
        } else {
            Vec::new()
        };
        let (c, refused) = self.candidates(&[disk.to_string()], &confirmed);
        if !refused.is_empty() {
            return Err(refused);
        }
        if let Some(why) = c.first().and_then(|c| c.taken.clone()) {
            return Err(one(qvsmsg::DISK_TAKEN.with(&[disk, &why])));
        }
        Ok(())
    }

    fn replace_unit(
        &mut self,
        name: &str,
        unit: u32,
        disk: &str,
        confirm: bool,
    ) -> Result<(), Refused> {
        let i = self
            .pool_index(name)
            .ok_or_else(|| one(qvsmsg::SPOOL_MISSING.with(&[name])))?;
        let member = usize::try_from(unit.saturating_sub(1)).unwrap_or(usize::MAX);
        let state = self.pools[i].label.members().nth(member).map(|u| u.state);
        let unit_text = unit.to_string();
        match state {
            None => {
                return Err(one(qvsmsg::UNIT_NOT_REPLACEABLE.with(&[
                    name,
                    &unit_text,
                    "the pool has no such unit",
                ])));
            }
            Some(UnitState::Failed | UnitState::Missing) => {}
            Some(s) => {
                return Err(one(qvsmsg::UNIT_NOT_REPLACEABLE.with(&[
                    name,
                    &unit_text,
                    &format!("it is {}", s.word()),
                ])));
            }
        }
        self.take_disk(disk, confirm)?;
        let small = self.pools[i].label.member_blocks;
        if plan::member_blocks(self.names(), loadsource::disks::disk_bytes(disk))
            .is_none_or(|b| b < small)
        {
            return Err(one(qvsmsg::DISK_TOO_SMALL.with(&[disk])));
        }
        let names = self.names();
        let found = create::prepare(names, disk)
            .map_err(|e| one(qvsmsg::UNIT_NOT_REPLACEABLE.with(&[name, &unit_text, &e])))?;
        let inventory = self.inventory();
        let identity = |d: &str| {
            let (_, serial, location) = super::disk_facts(&inventory, d);
            (serial, location)
        };
        create::replace(
            names,
            &mut self.pools[i],
            member,
            &found,
            &identity,
            clock(),
        )
        .map_err(|e| one(qvsmsg::UNIT_NOT_REPLACEABLE.with(&[name, &unit_text, &e])))?;
        self.post(
            &refcode::POOL_REBUILD_STARTED,
            &format!("{name}, unit {unit}, disk {disk}"),
        );
        Ok(())
    }

    fn add_spare(&mut self, name: &str, disk: &str, confirm: bool) -> Result<(), Refused> {
        let i = self
            .pool_index(name)
            .ok_or_else(|| one(qvsmsg::SPOOL_MISSING.with(&[name])))?;
        if self.pools[i].label.protection == Protection::None {
            return Err(one(qvsmsg::SPOOL_SPARES.with(&[
                disk,
                "a pool with no protection has no use for a hot spare",
            ])));
        }
        if self.pools[i].label.spares().count() >= plan::MAX_SPARES {
            return Err(one(qvsmsg::SPOOL_SPARES.with(&[
                disk,
                &format!("a pool has {} hot spares at most", plan::MAX_SPARES),
            ])));
        }
        self.take_disk(disk, confirm)?;
        let small = self.pools[i].label.member_blocks;
        if plan::member_blocks(self.names(), loadsource::disks::disk_bytes(disk))
            .is_none_or(|b| b < small)
        {
            return Err(one(qvsmsg::SPARE_TOO_SMALL.with(&[disk])));
        }
        let names = self.names();
        let found = create::prepare(names, disk)
            .map_err(|e| one(qvsmsg::SPOOL_SPARES.with(&[disk, &e])))?;
        let inventory = self.inventory();
        let identity = |d: &str| {
            let (_, serial, location) = super::disk_facts(&inventory, d);
            (serial, location)
        };
        create::add_spare(names, &mut self.pools[i], found, &identity, clock());
        Ok(())
    }

    fn remove_spare(&mut self, name: &str, disk: &str) -> Result<(), Refused> {
        let i = self
            .pool_index(name)
            .ok_or_else(|| one(qvsmsg::SPOOL_MISSING.with(&[name])))?;
        let names = self.names();
        create::remove_spare(names, &mut self.pools[i], disk, clock())
            .map_err(|e| one(qvsmsg::SPOOL_SPARES.with(&[disk, &e])))
    }

    fn designate(&mut self, name: &str) -> Result<(), Refused> {
        let target = if name.is_empty() {
            None
        } else {
            Some(self.usable_pool(name)?.mount.clone().unwrap_or_default())
        };
        if let Some(current) = self.repository()
            && Some(current.as_str()) != Some(name)
        {
            let held = self.holders.iter().any(|h| h.want.kind != Want_::Vdisk);
            if held {
                return Err(one(qvsmsg::MEDIA_HELD.with(&[
                    "The media library",
                    &self
                        .holders
                        .iter()
                        .find(|h| h.want.kind != Want_::Vdisk)
                        .map_or(0, |h| h.partition)
                        .to_string(),
                ])));
            }
            if let Some(at) = self
                .pools
                .iter()
                .find(|p| p.label.name == current)
                .and_then(|p| p.mount.clone())
            {
                let _ = fs::remove_file(at.join(media::DIR).join(DESIGNATED));
            }
        }
        if let Some(at) = target {
            let l = media::Library::new(&at);
            l.prepare()
                .and_then(|()| {
                    fs::write(at.join(media::DIR).join(DESIGNATED), b"")?;
                    vdisk::sync_dir(&at.join(media::DIR));
                    Ok(())
                })
                .map_err(|e| one(storage("designate the repository", &e)))?;
        }
        Ok(())
    }

    fn vdisk_list(&self, pool: &str) -> Vec<Vdisk> {
        self.pools
            .iter()
            .filter(|p| pool.is_empty() || p.label.name == pool)
            .filter_map(|p| Some((p.label.name.clone(), p.mount.clone()?)))
            .flat_map(|(name, at)| {
                vdisk::list(&at)
                    .into_iter()
                    .map(|d| Vdisk {
                        holder: holder_of(self, Want_::Vdisk, &name, &d.name),
                        pool: name.clone(),
                        name: d.name,
                        bytes: d.bytes,
                        allocated: d.allocated,
                        thick: d.kind == vdisk::Kind::Thick,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn vdisk_error(pool: &str, name: &str, mib: u64, e: &vdisk::VdiskError) -> Issued {
        use vdisk::VdiskError as E;
        match e {
            E::Name => qvsmsg::VDISK_NAME_INVALID.with(&[name]),
            E::Size => qvsmsg::VDISK_SIZE_INVALID.with(&[
                "1",
                &(vdisk::MAX_BYTES >> 20).to_string(),
                &mib.to_string(),
            ]),
            E::Exists => qvsmsg::VDISK_EXISTS.with(&[pool, name]),
            E::Missing => qvsmsg::VDISK_MISSING.with(&[pool, name]),
            E::Shrink => qvsmsg::VDISK_SHRINK.with(&[name, &mib.to_string()]),
            E::NoSpace => qvsmsg::SPOOL_NO_ROOM.with(&[pool, &mib.to_string()]),
            E::Io(io) => storage("change the virtual disk", io),
        }
    }

    fn create_vdisk(
        &self,
        pool: &str,
        name: &str,
        mib: u64,
        thick: bool,
    ) -> Result<Vdisk, Refused> {
        let at = self.usable_pool(pool)?.mount.clone().unwrap_or_default();
        let kind = if thick {
            vdisk::Kind::Thick
        } else {
            vdisk::Kind::Thin
        };
        let d = vdisk::create(&at, name, mib.saturating_mul(1 << 20), kind)
            .map_err(|e| one(Self::vdisk_error(pool, name, mib, &e)))?;
        Ok(Vdisk {
            pool: pool.into(),
            name: d.name,
            bytes: d.bytes,
            allocated: d.allocated,
            thick,
            holder: Holder::default(),
        })
    }

    fn resize_vdisk(&self, pool: &str, name: &str, mib: u64) -> Result<Vdisk, Refused> {
        let at = self.usable_pool(pool)?.mount.clone().unwrap_or_default();
        let h = holder_of(self, Want_::Vdisk, pool, name);
        if h.partition != 0 {
            return Err(one(
                qvsmsg::VDISK_HELD.with(&[name, &h.partition.to_string()])
            ));
        }
        let d = vdisk::resize(&at, name, mib.saturating_mul(1 << 20))
            .map_err(|e| one(Self::vdisk_error(pool, name, mib, &e)))?;
        Ok(Vdisk {
            pool: pool.into(),
            name: d.name,
            bytes: d.bytes,
            allocated: d.allocated,
            thick: d.kind == vdisk::Kind::Thick,
            holder: Holder::default(),
        })
    }

    fn delete_vdisk(&self, pool: &str, name: &str, force: bool) -> Result<(), Refused> {
        let at = self.usable_pool(pool)?.mount.clone().unwrap_or_default();
        let h = holder_of(self, Want_::Vdisk, pool, name);
        if h.partition != 0 {
            return Err(one(
                qvsmsg::VDISK_HELD.with(&[name, &h.partition.to_string()])
            ));
        }
        if !force {
            let named = (self.host.named)(pool, name);
            if !named.is_empty() {
                return Err(named
                    .iter()
                    .map(|n| qvsmsg::VDISK_NAMED.with(&[name, n]))
                    .collect());
            }
        }
        vdisk::delete(&at, name).map_err(|e| one(Self::vdisk_error(pool, name, 0, &e)))
    }

    fn image_list(&self) -> Vec<Image> {
        let Ok(l) = self.repository_library() else {
            return Vec::new();
        };
        l.images()
            .into_iter()
            .map(|i| Image {
                holder: holder_of(self, Want_::Image, "", &i.name),
                name: i.name,
                bytes: i.bytes,
                label: i.label,
            })
            .collect()
    }

    fn volume_list(&self) -> Vec<Volume> {
        let Ok(l) = self.repository_library() else {
            return Vec::new();
        };
        l.volumes()
            .into_iter()
            .map(|v| Volume {
                holder: holder_of(self, Want_::Volume, "", &v.name),
                name: v.name,
                identifier: v.volume.identifier,
                capacity_mib: v.volume.capacity_mib,
                used: v.used,
                protected: v.volume.write_protected,
            })
            .collect()
    }

    fn media_error(kind: &str, name: &str, e: &media::MediaError) -> Issued {
        use media::MediaError as E;
        match e {
            E::Name => qvsmsg::MEDIA_NAME_INVALID.with(&[name]),
            E::Exists => qvsmsg::MEDIA_EXISTS.with(&[kind, name]),
            E::Missing => qvsmsg::MEDIA_MISSING.with(&[kind, name]),
            E::NotIso => qvsmsg::NOT_ISO.with(&[name]),
            E::Size => qvsmsg::UPLOAD_SIZE.with(&[name]),
            E::Volume(why) => qvsmsg::VOLUME_INVALID.with(&[name, why]),
            E::Io(io) => storage("change the media library", io),
        }
    }

    fn not_held(&self, kind: Want_, name: &str, what: &str) -> Result<(), Refused> {
        let h = holder_of(self, kind, "", name);
        if h.partition != 0 {
            return Err(one(
                qvsmsg::MEDIA_HELD.with(&[&format!("{what} {name}"), &h.partition.to_string()])
            ));
        }
        Ok(())
    }

    /// Have `partition` hold `wants`: every one given, or none.
    fn hold(&mut self, partition: u32, wants: &[Want]) -> Result<Vec<(Given, File)>, Refused> {
        let mut refused = Vec::new();
        let mut given = Vec::new();
        for w in wants {
            let other = self
                .holders
                .iter()
                .find(|h| {
                    h.partition != partition
                        && h.want.kind == w.kind
                        && h.want.pool == w.pool
                        && h.want.name == w.name
                })
                .map(|h| h.partition);
            let opened: Result<(Given, File), Issued> = match w.kind {
                Want_::Vdisk => match (other, self.usable_pool(&w.pool)) {
                    (Some(id), _) => Err(qvsmsg::VDISK_HELD.with(&[&w.name, &id.to_string()])),
                    (None, Err(mut r)) => Err(r.remove(0)),
                    (None, Ok(p)) => {
                        let at = p.mount.clone().unwrap_or_default();
                        vdisk::open(&at, &w.name)
                            .map_err(|e| Self::vdisk_error(&w.pool, &w.name, 0, &e))
                            .map(|f| {
                                let bytes = f.metadata().map_or(0, |m| m.len());
                                (
                                    Given {
                                        want: w.clone(),
                                        bytes,
                                        ..Given::default()
                                    },
                                    f,
                                )
                            })
                    }
                },
                // An image is read only: partitions may share it.
                Want_::Image => match self.repository_library() {
                    Err(mut r) => Err(r.remove(0)),
                    Ok(l) => l
                        .open_image(&w.name)
                        .map_err(|e| Self::media_error("image", &w.name, &e))
                        .map(|f| {
                            let bytes = f.metadata().map_or(0, |m| m.len());
                            (
                                Given {
                                    want: w.clone(),
                                    bytes,
                                    ..Given::default()
                                },
                                f,
                            )
                        }),
                },
                Want_::Volume => match (other, self.repository_library()) {
                    (Some(id), _) => Err(qvsmsg::MEDIA_HELD
                        .with(&[&format!("tape volume {}", w.name), &id.to_string()])),
                    (None, Err(mut r)) => Err(r.remove(0)),
                    (None, Ok(l)) => l
                        .open_volume(&w.name)
                        .map_err(|e| Self::media_error("tape volume", &w.name, &e))
                        .map(|(f, v)| {
                            let bytes = f.metadata().map_or(0, |m| m.len());
                            (
                                Given {
                                    want: w.clone(),
                                    bytes,
                                    capacity_mib: v.capacity_mib,
                                    protected: v.write_protected,
                                },
                                f,
                            )
                        }),
                },
            };
            match opened {
                Ok(g) => given.push(g),
                Err(m) => refused.push(m),
            }
        }
        if !refused.is_empty() {
            return Err(refused);
        }
        for (g, _) in &given {
            self.holders.retain(|h| {
                !(h.partition == partition
                    && h.want.kind == g.want.kind
                    && h.want.slot == g.want.slot)
            });
            self.holders.push(Held {
                partition,
                want: g.want.clone(),
            });
        }
        write_holders(self);
        Ok(given)
    }

    fn release(&mut self, partition: u32, wants: &[Want]) {
        self.holders.retain(|h| {
            h.partition != partition
                || (!wants.is_empty()
                    && !wants
                        .iter()
                        .any(|w| w.kind == h.want.kind && w.slot == h.want.slot))
        });
        write_holders(self);
    }
}

fn storage(what: &str, e: &dyn std::fmt::Display) -> Issued {
    qvsmsg::STORAGE_FAILED.with(&[what, &said(e)])
}

/// The socket's requests on the service.
struct Session<'a> {
    core: &'a Shared,
}

impl proto::Pools for Session<'_> {
    fn disks(&mut self) -> Vec<Disk> {
        lock(self.core).disk_list()
    }
    fn candidates(&mut self) -> Vec<Disk> {
        lock(self.core)
            .disk_list()
            .into_iter()
            .filter(|d| d.free || d.foreign)
            .collect()
    }
    fn pools(&mut self) -> Vec<PoolStatus> {
        lock(self.core).pool_list()
    }
    fn create_pool(
        &mut self,
        name: String,
        protection: String,
        members: Vec<String>,
        spares: Vec<String>,
        confirm: Vec<String>,
    ) -> Result<PoolStatus, Refused> {
        lock(self.core).create_pool(&name, &protection, &members, &spares, &confirm)
    }
    fn delete_pool(&mut self, name: String) -> Result<(), Refused> {
        lock(self.core).delete_pool(&name)
    }
    fn rename_pool(&mut self, name: String, to: String) -> Result<(), Refused> {
        lock(self.core).rename_pool(&name, &to)
    }
    fn replace_unit(
        &mut self,
        pool: String,
        unit: u32,
        disk: String,
        confirm: bool,
    ) -> Result<(), Refused> {
        lock(self.core).replace_unit(&pool, unit, &disk, confirm)
    }
    fn add_spare(&mut self, pool: String, disk: String, confirm: bool) -> Result<(), Refused> {
        lock(self.core).add_spare(&pool, &disk, confirm)
    }
    fn remove_spare(&mut self, pool: String, disk: String) -> Result<(), Refused> {
        lock(self.core).remove_spare(&pool, &disk)
    }
    fn designate(&mut self, pool: String) -> Result<(), Refused> {
        lock(self.core).designate(&pool)
    }
    fn vdisks(&mut self, pool: String) -> Vec<Vdisk> {
        lock(self.core).vdisk_list(&pool)
    }
    fn create_vdisk(
        &mut self,
        pool: String,
        name: String,
        mib: u64,
        thick: bool,
    ) -> Result<Vdisk, Refused> {
        lock(self.core).create_vdisk(&pool, &name, mib, thick)
    }
    fn resize_vdisk(&mut self, pool: String, name: String, mib: u64) -> Result<Vdisk, Refused> {
        lock(self.core).resize_vdisk(&pool, &name, mib)
    }
    fn delete_vdisk(&mut self, pool: String, name: String, force: bool) -> Result<(), Refused> {
        lock(self.core).delete_vdisk(&pool, &name, force)
    }
    fn images(&mut self) -> Vec<Image> {
        lock(self.core).image_list()
    }
    fn upload(&mut self, name: String, _bytes: u64) -> Result<(), Refused> {
        // Carried by the connection itself; a call here has no bytes.
        Err(one(qvsmsg::UPLOAD_SIZE.with(&[&name])))
    }
    fn rename_image(&mut self, name: String, to: String) -> Result<(), Refused> {
        let c = lock(self.core);
        c.not_held(Want_::Image, &name, "image")?;
        let l = c.repository_library()?;
        l.rename_image(&name, &to)
            .map_err(|e| one(Core::media_error("image", &name, &e)))
    }
    fn delete_image(&mut self, name: String) -> Result<(), Refused> {
        let c = lock(self.core);
        c.not_held(Want_::Image, &name, "image")?;
        let l = c.repository_library()?;
        l.delete_image(&name)
            .map_err(|e| one(Core::media_error("image", &name, &e)))
    }
    fn import_image(&mut self, device: String, path: String, name: String) -> Result<(), Refused> {
        start_import(self.core, &device, &path, &name)
    }
    fn volumes(&mut self) -> Vec<Volume> {
        lock(self.core).volume_list()
    }
    fn create_volume(
        &mut self,
        name: String,
        identifier: String,
        mib: u64,
    ) -> Result<Volume, Refused> {
        let c = lock(self.core);
        let l = c.repository_library()?;
        l.create_volume(&name, &media::new_volume(&identifier, mib, clock()))
            .map_err(|e| one(Core::media_error("tape volume", &name, &e)))?;
        Ok(c.volume_list()
            .into_iter()
            .find(|v| v.name == name)
            .unwrap_or_default())
    }
    fn delete_volume(&mut self, name: String) -> Result<(), Refused> {
        let c = lock(self.core);
        c.not_held(Want_::Volume, &name, "tape volume")?;
        let l = c.repository_library()?;
        l.delete_volume(&name)
            .map_err(|e| one(Core::media_error("tape volume", &name, &e)))
    }
    fn protect_volume(&mut self, name: String, on: bool) -> Result<(), Refused> {
        let c = lock(self.core);
        c.not_held(Want_::Volume, &name, "tape volume")?;
        let l = c.repository_library()?;
        l.protect_volume(&name, on)
            .map_err(|e| one(Core::media_error("tape volume", &name, &e)))
    }
    fn hold(&mut self, _partition: u32, _wants: Vec<Want>) -> Result<Vec<Given>, Refused> {
        // Carried by the connection itself, with the files.
        Err(one(
            qvsmsg::STORAGE_FAILED.with(&["hold", "no files can be sent"])
        ))
    }
    fn release(&mut self, partition: u32, wants: Vec<Want>) {
        lock(self.core).release(partition, &wants);
    }
    fn holders(&mut self) -> Vec<Held> {
        lock(self.core).holders.clone()
    }
    fn stop_all(&mut self) -> u32 {
        lock(self.core).stop_all()
    }
}

fn answer(stream: &UnixStream, bytes: &[u8]) -> io::Result<()> {
    let mut s = stream;
    ipc::frame::write_frame(&mut s, bytes).map_err(io::Error::other)
}

/// One connection: a request and its answer.
pub fn connection(core: &Shared, stream: &UnixStream) {
    let mut s = stream;
    let Ok(request) = ipc::frame::read_frame_max(&mut s, proto::MAX_FRAME) else {
        return;
    };
    let Ok(request) = ipc::decode::<Request>(&request) else {
        return;
    };
    let done = match request {
        Request::Hold { partition, wants } => {
            let held = lock(core).hold(partition, &wants);
            match held {
                Ok(given) => {
                    let list: Vec<Given> = given.iter().map(|(g, _)| g.clone()).collect();
                    let answered = answer(stream, &ipc::encode(&Ok::<_, Refused>(list)));
                    let fds: Vec<_> = given.iter().map(|(_, f)| f.as_fd()).collect();
                    answered.and_then(|()| {
                        if fds.is_empty() {
                            Ok(())
                        } else {
                            fdpass::send_all(stream, &fds)
                        }
                    })
                }
                Err(refused) => answer(stream, &ipc::encode(&Err::<Vec<Given>, Refused>(refused))),
            }
        }
        Request::Upload { name, bytes } => upload(core, stream, &name, bytes),
        other => answer(stream, &proto::dispatch(&mut Session { core }, other)),
    };
    if let Err(e) = done {
        eprintln!("a connection: {e}");
    }
}

/// An upload: the bytes follow the first answer, the second says how
/// it went. Nothing is under the image's name until it is whole.
fn upload(core: &Shared, stream: &UnixStream, name: &str, bytes: u64) -> io::Result<()> {
    let begun = lock(core).repository_library().and_then(|l| {
        l.import(name, bytes)
            .map_err(|e| one(Core::media_error("image", name, &e)))
    });
    let mut import = match begun {
        Ok(import) => import,
        Err(refused) => return answer(stream, &ipc::encode(&Err::<(), Refused>(refused))),
    };
    answer(stream, &ipc::encode(&Ok::<(), Refused>(())))?;
    let mut buf = vec![0u8; CHUNK];
    let mut left = bytes;
    let mut s = stream;
    while left > 0 {
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let n = s.read(&mut buf[..want])?;
        if n == 0 {
            // Cut off: the import's file goes as it is dropped.
            return Ok(());
        }
        if let Err(e) = import.write(&buf[..n]) {
            let r = one(Core::media_error("image", name, &e));
            return answer(stream, &ipc::encode(&Err::<Image, Refused>(r)));
        }
        left -= n as u64;
    }
    let done = import.finish();
    let result = match done {
        Ok(i) => {
            lock(core).post(
                &refcode::MEDIA_IMPORTED,
                &format!("{}, {}, {} bytes", i.name, i.label, i.bytes),
            );
            Ok(Image {
                name: i.name,
                bytes: i.bytes,
                label: i.label,
                holder: Holder::default(),
            })
        }
        Err(e) => {
            let m = Core::media_error("image", name, &e);
            lock(core).post(&refcode::MEDIA_NOT_IMPORTED, &format!("{name}: {m}"));
            Err(one(m))
        }
    };
    answer(stream, &ipc::encode(&result))
}

/// Begin an import from a device of the host: the whole of an optical
/// disc (`path` empty), or a file of a removable volume. It runs on a
/// thread of its own; the pools' list does not show it, the history
/// does (`C2007050`, `A2007060`).
fn start_import(core: &Shared, device: &str, path: &str, name: &str) -> Result<(), Refused> {
    let mut c = lock(core);
    if c.import.is_some() {
        return Err(one(qvsmsg::IMPORT_BUSY.with(&[])));
    }
    let library = c.repository_library()?;
    if !media::valid_name(name) {
        return Err(one(qvsmsg::MEDIA_NAME_INVALID.with(&[name])));
    }
    if !device.bytes().all(|b| b.is_ascii_alphanumeric()) || device.is_empty() {
        return Err(one(qvsmsg::DISK_MISSING.with(&[device])));
    }
    c.import = Some((name.to_string(), 0, 0, String::new()));
    drop(c);
    let (core, device, path, name) = (
        core.clone(),
        device.to_string(),
        path.to_string(),
        name.to_string(),
    );
    let run_dir = lock(&core).host.run_dir;
    let _ = std::thread::Builder::new()
        .name("import".into())
        .spawn(move || {
            let done = import_from(&library, run_dir, &device, &path, &name);
            let mut c = lock(&core);
            c.import = None;
            match done {
                Ok(i) => c.post(
                    &refcode::MEDIA_IMPORTED,
                    &format!("{}, {}, {} bytes, from {device}", i.name, i.label, i.bytes),
                ),
                Err(e) => c.post(&refcode::MEDIA_NOT_IMPORTED, &format!("{name}: {e}")),
            }
        });
    Ok(())
}

fn import_from(
    library: &media::Library,
    run_dir: &str,
    device: &str,
    path: &str,
    name: &str,
) -> Result<media::Image, String> {
    let node = Path::new("/dev").join(device);
    if path.is_empty() {
        let mut f = File::open(&node).map_err(|e| format!("{}: {e}", node.display()))?;
        let bytes = loadsource::disks::disk_bytes(device);
        return copy_in(library, name, bytes, &mut f);
    }
    // A file of a removable volume: mounted read only for the copy.
    let at = Path::new(run_dir).join("poold").join("import");
    fs::create_dir_all(&at).map_err(said)?;
    let flags = rustix::mount::MountFlags::RDONLY
        | rustix::mount::MountFlags::NODEV
        | rustix::mount::MountFlags::NOSUID
        | rustix::mount::MountFlags::NOEXEC;
    let mounted = ["vfat", "iso9660", "exfat", "ext4", "xfs"]
        .iter()
        .any(|fs| rustix::mount::mount(&node, &at, *fs, flags, None).is_ok());
    if !mounted {
        return Err(format!(
            "{} holds no volume that can be read",
            node.display()
        ));
    }
    let relative = path.trim_start_matches('/');
    let result = if relative.split('/').any(|p| p == "..") {
        Err("a path within the volume".to_string())
    } else {
        let file = at.join(relative);
        File::open(&file)
            .map_err(|e| format!("{path}: {e}"))
            .and_then(|mut f| {
                let bytes = f.metadata().map_err(said)?.len();
                copy_in(library, name, bytes, &mut f)
            })
    };
    let _ = rustix::mount::unmount(&at, rustix::mount::UnmountFlags::empty());
    result
}

fn copy_in(
    library: &media::Library,
    name: &str,
    bytes: u64,
    source: &mut dyn Read,
) -> Result<media::Image, String> {
    let mut import = library.import(name, bytes).map_err(said)?;
    let mut buf = vec![0u8; CHUNK];
    let mut left = bytes;
    while left > 0 {
        let want = usize::try_from(left.min(CHUNK as u64)).unwrap_or(CHUNK);
        let n = source.read(&mut buf[..want]).map_err(said)?;
        if n == 0 {
            return Err("the source ended early".into());
        }
        import.write(&buf[..n]).map_err(said)?;
        left -= n as u64;
    }
    import.finish().map_err(said)
}
