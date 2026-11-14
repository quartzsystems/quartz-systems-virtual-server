// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Making a pool and changing its units: disks made pool disks (a GPT
//! with a record partition and a member partition), the label written
//! to each, the set built, the file system made and mounted; a unit
//! put in a failed one's place; hot spares added and taken away; a
//! pool renamed or deleted.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use blockdev::{BlockDev, LinuxBlockDev};
use diskimg::layout::LayoutGuids;
use diskimg::xfsfmt;
use loadsource::disks;
use pool::label::{PoolLabel, Protection, Unit, UnitState};
use pool::layout;
use pool::names::PoolDiskNames;
use pool::plan::Plan;
use protect::parity::Level;
use protect::set::ParitySet;
use protect::spare::Spare;
use uuid::Uuid;

use super::runtime::{self, Found, Pool, Set, Work};

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Sixteen random bytes from the kernel.
pub fn random() -> Result<[u8; 16], String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    Ok(bytes)
}

fn uuid() -> Result<Uuid, String> {
    Ok(uuid::Builder::from_random_bytes(random()?).into_uuid())
}

/// Whether anything of `disk` is mounted.
pub fn mounted(disk: &str) -> bool {
    let prefix = format!("/dev/{disk}");
    std::fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .any(|l| l.split(' ').next().is_some_and(|d| d.starts_with(&prefix)))
}

/// Make `disk` a pool disk: its table written, its record partition
/// clear. The disk as found afterwards.
pub fn prepare(names: &PoolDiskNames, disk: &str) -> Result<Found, String> {
    let bytes = disks::disk_bytes(disk);
    let l = layout::layout(names, bytes)?;
    let guids = LayoutGuids {
        disk: uuid()?,
        parts: vec![uuid()?, uuid()?],
    };
    let node = Path::new("/dev").join(disk);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)
        .map_err(|e| format!("{}: {e}", node.display()))?;
    diskimg::write_layout(&mut file, &l, &guids, &[], &mut |_, _| {}).map_err(text)?;
    // The whole record partition cleared: no label of an earlier life.
    let record = l.parts[0].part.offset();
    file.seek(SeekFrom::Start(record)).map_err(text)?;
    file.write_all(&vec![
        0u8;
        usize::try_from(layout::RECORD_BYTES).unwrap_or(0)
    ])
    .map_err(text)?;
    file.sync_all().map_err(text)?;
    loadsource::sys::reread_partitions(&file)
        .map_err(|e| format!("{disk}: the table was not read: {e}"))?;
    drop(file);
    let give_up = Instant::now() + Duration::from_secs(10);
    loop {
        let nodes = (
            disks::partition_node(disk, 1),
            disks::partition_node(disk, 2),
        );
        if let (Some(record), Some(member)) = nodes
            && record.exists()
            && member.exists()
        {
            return Ok(Found {
                disk: disk.to_string(),
                record,
                member,
                partition: guids.parts[1],
            });
        }
        if Instant::now() > give_up {
            return Err(format!("the partitions of {disk} did not appear"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Clear what makes `disk` a pool disk: its table and its record
/// partition's head, so it is not configured afterwards.
pub fn release_disk(found: &Found) {
    if let Ok(mut f) = OpenOptions::new().write(true).open(&found.record) {
        let _ = f.write_all(&vec![0u8; 256 * 1024]);
        let _ = f.sync_all();
    }
    let node = Path::new("/dev").join(&found.disk);
    if let Ok(mut f) = OpenOptions::new().write(true).open(&node) {
        let _ = f.write_all(&vec![0u8; 64 * 1024]);
        let _ = f.sync_all();
        let _ = loadsource::sys::reread_partitions(&f);
    }
}

/// Make the pool `plan` describes, called `name`, for `host`; mounted
/// at `at`. `ids` names the units' disks.
#[allow(clippy::too_many_arguments)] // each is one thing a pool is made of
pub fn create(
    names: &'static PoolDiskNames,
    host: [u8; 16],
    name: &str,
    plan: &Plan,
    identity: &dyn Fn(&str) -> (String, String),
    id_file: &Path,
    at: &Path,
    now: u64,
) -> Result<Pool, String> {
    let mut found = Vec::new();
    for disk in plan.members.iter().chain(&plan.spares) {
        found.push(prepare(names, disk)?);
    }
    let unit = |f: &Found, state| {
        let (serial, location) = identity(&f.disk);
        Unit {
            partition: f.partition,
            state,
            serial,
            location,
        }
    };
    let n = plan.members.len();
    let units: Vec<Unit> = found
        .iter()
        .enumerate()
        .map(|(i, f)| {
            unit(
                f,
                if i < n {
                    UnitState::Active
                } else {
                    UnitState::Spare
                },
            )
        })
        .collect();
    let pool_id = uuid()?;
    let mut label = PoolLabel {
        host,
        pool: pool_id,
        name: name.to_string(),
        protection: plan.protection,
        set_blocks: plan.set_blocks,
        member_blocks: plan.member_blocks,
        parity: plan.geometry.map_or((0, 0), |g| (g.blocks, g.head)),
        sync: None,
        clean: true,
        written: now,
        generation: 1,
        units,
    };
    let mut p = Pool::new(label.clone(), found.clone());
    match plan.protection {
        Protection::Raid5 | Protection::Raid6 => {
            let level = if plan.protection == Protection::Raid6 {
                Level::Raid6
            } else {
                Level::Raid5
            };
            let devs = found[..n]
                .iter()
                .map(|f| {
                    let dev = LinuxBlockDev::open(&f.member).map_err(text)?;
                    Ok((*f.partition.as_bytes(), Box::new(dev) as Box<dyn BlockDev>))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let set =
                ParitySet::create(runtime::format(names), *pool_id.as_bytes(), level, 0, devs)
                    .map_err(text)?;
            label.parity = (set.geometry().blocks, set.geometry().head);
            label.set_blocks = set.blocks();
            label.sync = set.sync();
            p.label = label.clone();
            p.recorded.set(Set::Parity(set.clone()).states());
            p.set = Some(Set::Parity(set));
            p.spares = spares_of(&found[n..]);
        }
        _ => {
            super::records::write_label(names, &p.records(), &label);
            p.open(names, true)?;
        }
    }
    p.write_label(names, now, true);
    let node = p.serve(id_file)?;
    format(&node, label.set_blocks, name, now)?;
    p.mount_at(&node, at)?;
    std::fs::create_dir_all(at.join(vdisk::DIR)).map_err(text)?;
    p.label.clean = false;
    p.write_label(names, now, true);
    Ok(p)
}

fn spares_of(found: &[Found]) -> Vec<Spare> {
    found
        .iter()
        .filter_map(|f| {
            Some(Spare {
                uuid: *f.partition.as_bytes(),
                dev: Box::new(LinuxBlockDev::open(&f.member).ok()?),
            })
        })
        .collect()
}

/// Make the pool's xfs file system on its device.
fn format(node: &Path, blocks: u64, name: &str, now: u64) -> Result<(), String> {
    let mut dev = OpenOptions::new()
        .read(true)
        .write(true)
        .open(node)
        .map_err(|e| format!("{}: {e}", node.display()))?;
    let mut options = xfsfmt::Options::new(
        uuid()?,
        xfsfmt::Timestamp {
            secs: i64::try_from(now).unwrap_or(0),
            nsecs: 0,
        },
    );
    options.label = name.chars().take(12).collect();
    options.sector_size = 4096;
    let bytes = (blocks * 4096).min(xfsfmt::MAX_BYTES);
    xfsfmt::format(&mut dev, bytes, &options).map_err(text)?;
    dev.sync_all().map_err(text)
}

/// Put the disk `found` in the place of unit `member` (from 0) and
/// start its rebuild.
pub fn replace(
    names: &'static PoolDiskNames,
    p: &mut Pool,
    member: usize,
    found: &Found,
    identity: &dyn Fn(&str) -> (String, String),
    now: u64,
) -> Result<(), String> {
    let set = p.set.clone().ok_or("the pool is not assembled")?;
    let dev = LinuxBlockDev::open(&found.member).map_err(text)?;
    let uuid = *found.partition.as_bytes();
    let (serial, location) = identity(&found.disk);
    let gone = p.label.units.get(member).map(|u| u.partition);
    p.found.retain(|f| Some(f.partition) != gone);
    p.found.push(found.clone());
    match &set {
        Set::Mirror(m) => {
            m.replace(member, Box::new(dev), uuid).map_err(text)?;
        }
        Set::Parity(s) => {
            s.replace(member, Box::new(dev), uuid).map_err(text)?;
        }
        Set::Plain(_) => return Err("a pool with no protection has no unit to rebuild".into()),
    }
    if let Some(u) = p.label.units.get_mut(member) {
        u.partition = found.partition;
        u.serial = serial;
        u.location = location;
    }
    p.write_label(names, now, true);
    start_rebuild(names, p, member)
}

/// Begin copying to a mirrored pool's member `member`; a parity
/// pool's rebuild is the set's own work.
pub fn start_rebuild(
    names: &'static PoolDiskNames,
    p: &mut Pool,
    member: usize,
) -> Result<(), String> {
    if let Some(Set::Mirror(m)) = &p.set {
        let mut journal = runtime::journal(names, p);
        let resume = m.resume(member, &mut journal).map_err(text)?;
        p.work = Some(Work {
            member,
            resume,
            rebuild: true,
        });
    }
    Ok(())
}

/// Give a pool the hot spare `found`.
pub fn add_spare(
    names: &'static PoolDiskNames,
    p: &mut Pool,
    found: Found,
    identity: &dyn Fn(&str) -> (String, String),
    now: u64,
) {
    let (serial, location) = identity(&found.disk);
    p.label.units.push(Unit {
        partition: found.partition,
        state: UnitState::Spare,
        serial,
        location,
    });
    p.spares.extend(spares_of(std::slice::from_ref(&found)));
    p.found.push(found);
    p.write_label(names, now, true);
}

/// Take the hot spare on `disk` from a pool; the disk is not
/// configured afterwards.
pub fn remove_spare(
    names: &'static PoolDiskNames,
    p: &mut Pool,
    disk: &str,
    now: u64,
) -> Result<(), String> {
    let f = p
        .found
        .iter()
        .find(|f| f.disk == disk)
        .cloned()
        .ok_or_else(|| format!("{disk} is not a disk of the pool"))?;
    let is_spare = p
        .label
        .unit_of(f.partition)
        .is_some_and(|(_, u)| u.state == UnitState::Spare);
    if !is_spare {
        return Err(format!("{disk} is not a hot spare of the pool"));
    }
    let uuid = *f.partition.as_bytes();
    p.spares.retain(|s| s.uuid != uuid);
    p.label.units.retain(|u| u.partition != f.partition);
    p.found.retain(|x| x.partition != f.partition);
    p.write_label(names, now, true);
    release_disk(&f);
    Ok(())
}

/// The pool's mount point under `pools`.
pub fn mount_point(pools: &Path, name: &str) -> PathBuf {
    pools.join(name)
}
