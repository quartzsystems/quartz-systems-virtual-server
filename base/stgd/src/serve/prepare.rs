// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A disk made a load-source disk while the host runs: as the second
//! unit of a load source that had none, or in the place of a unit that
//! is missing or failed. The disk gets the table, a copy of the ESP
//! with both slots, the A/B record and the set's label; its data
//! partition is then written by the rebuild.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

use appliance::LoadSourceNames;
use blockdev::LinuxBlockDev;
use loadsource::boot::{self, EFIVARS};
use loadsource::disks::{self, LoadSource};
use loadsource::set::{Member, MemberState, SetLabel};
use loadsource::{prepare, record, sys};
use protect::UnitState;
use uuid::Uuid;

use super::{Core, Kind, What, blocks_of, text};

/// Sixteen random bytes from the kernel.
fn random() -> Result<[u8; 16], String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    Ok(bytes)
}

/// Whether anything of `disk` is mounted.
fn mounted(disk: &str) -> bool {
    let prefix = format!("/dev/{disk}");
    std::fs::read_to_string("/proc/mounts")
        .unwrap_or_default()
        .lines()
        .any(|l| l.split(' ').next().is_some_and(|d| d.starts_with(&prefix)))
}

/// `disk` can be erased and holds a data partition of `bytes`.
fn check_disk(core: &Core, disk: &str, bytes: u64) -> Result<(), String> {
    if !disks::whole_disks().iter().any(|d| d == disk) {
        return Err(format!("there is no disk {disk}"));
    }
    if mounted(disk) {
        return Err(format!("disk {disk} is in use"));
    }
    let in_step = core
        .units()
        .iter()
        .any(|u| u.disk == disk && (u.state == "Active" || u.state == "Unprotected"));
    if in_step {
        return Err(format!("disk {disk} is a unit of the load source"));
    }
    let layout = prepare::layout(core.names(), disks::disk_bytes(disk))
        .map_err(|e| format!("disk {disk} is too small: {e}"))?;
    let room = layout.parts.last().map_or(0, |p| p.part.bytes());
    if room < bytes {
        return Err(format!(
            "disk {disk} is too small: its data partition would be {room} bytes, the load source's is {bytes}"
        ));
    }
    Ok(())
}

/// Whether mirrored protection can be started with `disk`.
pub fn check_start(core: &Core, disk: &str) -> Result<(), String> {
    if core.label.is_some() || core.restart_needed {
        return Err("the load source is mirrored already".into());
    }
    if core.job.is_some() {
        return Err("another disk is being prepared".into());
    }
    let [source] = core.sources.as_slice() else {
        return Err(format!(
            "{} load-source disks were found, not one",
            core.sources.len()
        ));
    };
    // The set's device has 4096-byte blocks; the file system must have
    // been made for them, as the installer makes it.
    let sector = File::open(&source.data)
        .ok()
        .and_then(|mut f| prepare::xfs_sector_size(&mut f));
    if sector != Some(4096) {
        return Err(
            "the data file system was not made by the installer and cannot be mirrored".into(),
        );
    }
    check_disk(core, disk, source.data_bytes)
}

/// Whether `disk` can take the place of unit `number`: its member.
pub fn check_replace(core: &Core, number: u32, disk: &str) -> Result<usize, String> {
    let (Some(label), Some(set)) = (&core.label, &core.set) else {
        return Err("the load source is not mirrored".into());
    };
    if core.job.is_some() || core.work.is_some() {
        return Err("a unit is being brought in step; wait for it".into());
    }
    let member = usize::try_from(number)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .filter(|m| *m < label.members.len())
        .ok_or_else(|| format!("there is no unit {number}"))?;
    let state = set.members()[member].state;
    if !matches!(
        state,
        UnitState::Missing | UnitState::NotOperational | UnitState::Suspended
    ) {
        return Err(format!("unit {number} is {state}, not missing or failed"));
    }
    check_disk(core, disk, label.blocks * loadsource::set::BLOCK)?;
    Ok(member)
}

/// Write the table and a copy of `from`'s ESP to `disk`; the disk as a
/// load source.
fn make(names: &LoadSourceNames, disk: &str, from: &LoadSource) -> Result<LoadSource, String> {
    let mut esp = Vec::new();
    File::open(&from.esp)
        .and_then(|mut f| f.read_to_end(&mut esp))
        .map_err(|e| format!("{}: {e}", from.esp.display()))?;
    let guids = prepare::guids_from(&[random()?, random()?, random()?, random()?]);
    let node = Path::new("/dev").join(disk);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&node)
        .map_err(|e| format!("{}: {e}", node.display()))?;
    prepare::write_table(
        names,
        &mut file,
        disks::disk_bytes(disk),
        &esp,
        &guids,
        &mut |_, _| {},
    )?;
    file.sync_all().map_err(text)?;
    sys::reread_partitions(&file).map_err(|e| format!("{disk}: the table was not read: {e}"))?;
    drop(file);
    let give_up = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(source) = disks::load_source(names, disk) {
            return Ok(source);
        }
        if Instant::now() > give_up {
            return Err(format!("the partitions of {disk} did not appear"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The new disk gets the A/B record the others have, and the firmware
/// an entry for each of its slots.
fn join(core: &mut Core, old: &[LoadSource]) -> Result<(), String> {
    let (current, _) = disks::record(core.names(), old).map_err(text)?;
    let mut records = disks::records(core.names(), &core.sources).map_err(text)?;
    record::write_all(&mut records, current).map_err(text)?;
    let esps: Vec<_> = core.sources.iter().map(|s| s.esp_at).collect();
    if let Err(e) = boot::make_default(
        Path::new(EFIVARS),
        core.names(),
        core.app.name,
        &esps,
        current.default_slot,
    ) {
        core.post(What::BootEntriesNotWritten, &e);
    }
    core.read_inventory();
    Ok(())
}

/// Start mirrored protection: `disk` becomes unit 2, to be written at
/// the next IPL, when the set is assembled.
pub fn start_protection(core: &mut Core, disk: &str) -> Result<(), String> {
    check_start(core, disk)?;
    let first = core.sources[0].clone();
    let second = make(core.names(), disk, &first)?;
    let old = core.sources.clone();
    core.sources = vec![first.clone(), second.clone()];
    join(core, &old)?;
    let member = |source: &LoadSource, state| {
        let (serial, location) = core.identity(&source.disk);
        Member {
            partition: source.data_guid,
            state,
            serial,
            location,
        }
    };
    let label = SetLabel {
        id: Uuid::from_bytes(random()?),
        blocks: blocks_of(first.data_bytes),
        // Nothing has run on it yet.
        clean: true,
        members: vec![
            member(&first, MemberState::Active),
            member(&second, MemberState::Resuming),
        ],
    };
    disks::labels(core.names(), &core.sources)
        .map_err(text)?
        .write(&label.encode())
        .map_err(text)?;
    core.restart_needed = true;
    core.post(
        What::ProtectionStarted,
        &format!("unit 2 is disk {disk}; the set is assembled at the next IPL"),
    );
    Ok(())
}

/// Put `disk` in the place of `member` and start its rebuild.
pub fn replace(core: &mut Core, member: usize, disk: &str) -> Result<(), String> {
    let number = u32::try_from(member + 1).unwrap_or(u32::MAX);
    check_replace(core, number, disk)?;
    let (Some(set), Some(label)) = (core.set.clone(), core.label.clone()) else {
        return Err("the load source is not mirrored".into());
    };
    // A unit in step lends its ESP.
    let from = label
        .members
        .iter()
        .zip(set.members())
        .filter(|(_, now)| now.state == UnitState::Active)
        .find_map(|(m, _)| core.source_of(m).cloned())
        .ok_or("no unit in step was found")?;
    let new = make(core.names(), disk, &from)?;
    let old = core.sources.clone();
    // The disk that was the member, if it is still there, is one no more.
    let gone = label.members[member].partition;
    core.sources
        .retain(|s| s.data_guid != gone && s.disk != new.disk);
    core.sources.push(new.clone());
    core.sources.sort_by(|a, b| a.disk.cmp(&b.disk));
    join(core, &old)?;
    let dev = LinuxBlockDev::open(&new.data).map_err(text)?;
    set.replace(member, Box::new(dev), *new.data_guid.as_bytes())
        .map_err(text)?;
    core.write_label(true);
    core.begin(member, Kind::Rebuild);
    Ok(())
}
