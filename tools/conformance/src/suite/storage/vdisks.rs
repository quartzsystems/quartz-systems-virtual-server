// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Virtual disks: thin and thick, a pool that fills under a running
//! partition, and who may hold, grow and delete a disk.

use std::time::{Duration, Instant};

use partd::Calls as _;
use poold::proto::{Calls as _, Vdisk};

use super::{POOL_DISK_BYTES, disk_names, host, pool_disks, pool_until, poold, said};
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::manager::{self, Spec};
use crate::suite::partition::{NAME, QSVS_MEMORY_MIB, guest_line, power_off, profile};

fn vdisk(guest: &mut Session, pool: &str, name: &str) -> Result<Vdisk, String> {
    poold(guest)?
        .vdisks(pool.to_string())
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| format!("no virtual disk {pool}/{name}"))
}

/// The host with a pool disk and a copy of the cheap guest's disk (the
/// release image, from side A) as further drives: their kernel
/// names.
fn with_guest_image<'a>(
    run: &Run<'a>,
) -> Result<(crate::suite::base::Guest<'a>, String, String), String> {
    let release = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    let disks = pool_disks(run, 1, POOL_DISK_BYTES)?;
    let image = crate::suite::partition::unit(run, release)?;
    let mut guest = host(run, &[disks[0].clone(), image])?;
    let names = disk_names(&mut guest, 2)?;
    Ok((guest, names[0].clone(), names[1].clone()))
}

/// Copy the guest's disk from its drive into a virtual disk's file.
fn copy_guest(guest: &mut Session, drive: &str, file: &str) -> Result<(), String> {
    let bytes = loadsource_bytes(guest, drive)?;
    channel(guest)?.ask_within(
        &format!("copy /dev/{drive} 0 {file} 0 {bytes}"),
        Duration::from_mins(5),
    )?;
    Ok(())
}

/// A drive's size, as the pool service lists it.
fn loadsource_bytes(guest: &mut Session, drive: &str) -> Result<u64, String> {
    poold(guest)?
        .disks()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.name == drive)
        .map(|d| d.bytes)
        .ok_or_else(|| format!("no disk {drive}"))
}

/// The cheap guest's profile over `backing`.
fn cheap(backing: &str) -> Spec {
    profile(1, QSVS_MEMORY_MIB, "side a", backing)
}

/// Fill pool `pool` with the file `name`: random bytes, which the
/// hooks' copy does not leave as holes, until the pool has no room.
fn fill(guest: &mut Session, pool: &str, name: &str) -> Result<(), String> {
    let p = pool_until(guest, pool, Instant::now(), |_| true)?;
    let room = p.bytes.saturating_sub(p.allocated) + (64 << 20);
    // Refused at the end, when the pool is full: that is the point.
    let _ = channel(guest)?.ask_within(
        &format!("copy /dev/urandom 0 /pools/{pool}/{name} 0 {room}"),
        Duration::from_mins(10),
    );
    let after = pool_until(guest, pool, Instant::now(), |_| true)?;
    if after.bytes.saturating_sub(after.allocated) > 16 << 20 {
        return Err(format!("the pool was not filled: {after:?}"));
    }
    Ok(())
}

/// A pool of one disk, made and mounted.
fn one_pool(guest: &mut Session, name: &str, disk: &str) -> Result<(), String> {
    said(poold(guest)?.create_pool(
        name.into(),
        "none".into(),
        vec![disk.to_string()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(
        guest,
        name,
        Instant::now() + Duration::from_secs(120),
        |p| !p.mount.is_empty(),
    )
    .map(|_| ())
}

/// `pool-thin-thick`: a thin disk takes space as it is written and
/// gives it back on discard; a thick disk has its space when made, and
/// its writes find room with the pool otherwise full.
pub fn thin_thick(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = pool_disks(run, 1, POOL_DISK_BYTES)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, 1)?;
    one_pool(&mut guest, "POOL1", &names[0])?;
    let thin = said(poold(&mut guest)?.create_vdisk("POOL1".into(), "THIN".into(), 1024, false))?;
    let thick = said(poold(&mut guest)?.create_vdisk("POOL1".into(), "THICK".into(), 256, true))?;
    if thin.allocated > 1 << 20 || thick.allocated < 256 << 20 {
        return Err(format!("as made: {thin:?}, {thick:?}"));
    }
    let file = "/pools/POOL1/vdisks/THIN.img";
    channel(&mut guest)?.ask(&format!("copy /dev/urandom 0 {file} 0 {}", 64 << 20))?;
    let written = vdisk(&mut guest, "POOL1", "THIN")?;
    if written.allocated < 64 << 20 {
        return Err(format!("written, the thin disk is {written:?}"));
    }
    // A discard returns the space: the same call a guest's UNMAP makes.
    channel(&mut guest)?.ask(&format!("punch {file} 0 {}", 64 << 20))?;
    let discarded = vdisk(&mut guest, "POOL1", "THIN")?;
    if discarded.allocated > 1 << 20 {
        return Err(format!("discarded, the thin disk is {discarded:?}"));
    }
    // The pool filled by a file of its own: the thick disk still writes.
    fill(&mut guest, "POOL1", "filler")?;
    let thick_file = "/pools/POOL1/vdisks/THICK.img";
    channel(&mut guest)?.ask_within(
        &format!("copy /dev/urandom 0 {thick_file} 0 {}", 255 << 20),
        Duration::from_mins(5),
    )?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "a thin disk of 1 GiB made with {} bytes allocated, {} after 64 MiB written and {} after \
         they were discarded; a thick disk of 256 MiB made with {} allocated, and written whole \
         with the pool otherwise full",
        thin.allocated, written.allocated, discarded.allocated, thick.allocated
    ))
}

/// `pool-full`: a pool that fills under a running partition: the
/// partition is held, with the attention code, no error reaches the
/// guest, and it goes on when there is room.
pub fn full(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let (mut guest, pool, image) = with_guest_image(run)?;
    one_pool(&mut guest, "POOL1", &pool)?;
    // The cheap guest's disk, copied into a thin disk larger than the
    // pool: it writes until the pool is full.
    said(poold(&mut guest)?.create_vdisk("POOL1".into(), "GUEST".into(), 4096, false))?;
    copy_guest(&mut guest, &image, "/pools/POOL1/vdisks/GUEST.img")?;
    // The rest of the pool taken: the guest's first write to a part of
    // its disk not yet written finds no room.
    fill(&mut guest, "POOL1", "filler")?;
    let id = manager::partition(&mut guest, NAME)?;
    let spec = cheap("POOL1/GUEST");
    manager::activate(&mut guest, id, &spec).map_err(|e| manager::refused(&e))?;
    guest.seen(&posted(&refcode::PARTITION_HELD, ""), run.deadline())?;
    // A guest with no service channel is Running once the wait for one
    // is over (Phase 7), held or not.
    let held = manager::until(&mut guest, id, partd::State::Running, run.deadline())?;
    if held.state != partd::State::Running {
        return Err(format!("held, the partition is {}", held.state.word()));
    }
    // Room again: the partition goes on by itself.
    channel(&mut guest)?.ask("rm /pools/POOL1/filler")?;
    guest.seen(&posted(&refcode::PARTITION_RESUMED, ""), run.deadline())?;
    if guest.said(&guest_line("I/O error")) {
        return Err("an I/O error reached the guest".into());
    }
    manager::said(manager::partd(&mut guest)?.shut_down(id))?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "a partition whose thin disk filled its pool: held with `{}`, still {}, no error in \
         the guest; `{}` when room was made",
        refcode::PARTITION_HELD.code,
        held.state.word(),
        refcode::PARTITION_RESUMED.code
    ))
}

/// `pool-vdisk-rules`: a disk held by one partition at a time; grown
/// only while no partition holds it; deleted only when none holds it
/// and, unless the request says so, no profile names it.
pub fn rules(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let (mut guest, pool, image) = with_guest_image(run)?;
    one_pool(&mut guest, "POOL1", &pool)?;
    said(poold(&mut guest)?.create_vdisk("POOL1".into(), "SHARED".into(), 1024, false))?;
    copy_guest(&mut guest, &image, "/pools/POOL1/vdisks/SHARED.img")?;
    let first = manager::partition(&mut guest, "FIRST")?;
    let second = manager::partition(&mut guest, "SECOND")?;
    let spec = cheap("POOL1/SHARED");
    manager::activate(&mut guest, first, &spec).map_err(|e| manager::refused(&e))?;
    let refused = manager::activate(&mut guest, second, &spec)
        .err()
        .ok_or("a second partition was given the disk")?;
    if !refused.iter().any(|m| m.is(&qvsmsg::VDISK_HELD)) {
        return Err(format!("the second activation said {refused:?}"));
    }
    let grow = poold(&mut guest)?
        .resize_vdisk("POOL1".into(), "SHARED".into(), 2048)
        .map_err(|e| e.to_string())?;
    if !grow.is_err_and(|m| m.iter().any(|m| m.is(&qvsmsg::VDISK_HELD))) {
        return Err("a held disk was grown".into());
    }
    manager::said(manager::partd(&mut guest)?.shut_down(first))?;
    manager::ended(&mut guest, first, run.deadline())?;
    said(poold(&mut guest)?.resize_vdisk("POOL1".into(), "SHARED".into(), 2048))?;
    // Named by both partitions' profiles: refused unless forced.
    let named = poold(&mut guest)?
        .delete_vdisk("POOL1".into(), "SHARED".into(), false)
        .map_err(|e| e.to_string())?;
    let Err(named) = named else {
        return Err("a disk two profiles name was deleted".into());
    };
    if named.iter().filter(|m| m.is(&qvsmsg::VDISK_NAMED)).count() != 2 {
        return Err(format!("the refusal said {named:?}"));
    }
    // Deleting a partition with its disks deletes them.
    said(poold(&mut guest)?.create_vdisk("POOL1".into(), "MINE".into(), 64, false))?;
    let mine = manager::partition(&mut guest, "MINE")?;
    manager::put(&mut guest, mine, &cheap("POOL1/MINE"))?;
    manager::said(manager::partd(&mut guest)?.delete(mine, true))?;
    if vdisk(&mut guest, "POOL1", "MINE").is_ok() {
        return Err("the deleted partition's disk is there".into());
    }
    said(poold(&mut guest)?.delete_vdisk("POOL1".into(), "SHARED".into(), true))?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(
        "one partition held the disk and a second's activation was refused (QVS3105); growing \
        it was refused while held and done after; deleting it was refused while two profiles \
        named it (QVS3108, each) and done when forced; a partition deleted with its disks took \
        them with it"
            .into(),
    )
}
