// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Pools: made, assembled again after a restart, their state file made
//! again from the labels; units pulled per protection, a hot spare
//! taking a unit's place; more units lost than a pool's protection
//! covers; the service killed; power pulled; disks that are not the
//! host's.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use poold::proto::{Calls as _, PoolStatus, state};

use super::{POOL_DISK_BYTES, disk_names, host, pool_disks, pool_until, poold, said};
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::power_off;

/// How long a new parity set's parity or a rebuild of a small pool may
/// take, three levels deep.
const WORK_WITHIN: Duration = Duration::from_secs(600);

fn active(p: &PoolStatus) -> bool {
    p.state == state::ACTIVE
}

/// A pool made of `disks`: the disks' kernel names.
fn make(
    guest: &mut crate::qemu::session::Session,
    name: &str,
    protection: &str,
    members: &[String],
    spares: &[String],
) -> Result<PoolStatus, String> {
    said(poold(guest)?.create_pool(
        name.to_string(),
        protection.to_string(),
        members.to_vec(),
        spares.to_vec(),
        Vec::new(),
    ))
}

/// `pool-create`: a mirrored pool made; the host restarted and the
/// pool assembled from its labels; its state file deleted and made
/// again from them.
pub fn create(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = pool_disks(run, 2, POOL_DISK_BYTES)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, 2)?;
    let made = make(&mut guest, "POOL1", "mirrored", &names, &[])?;
    let made = pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, active)
        .map_err(|e| format!("{e}; made as {made:?}"))?;
    if made.mount != "/pools/POOL1" || made.units.len() != 2 {
        return Err(format!("the pool is {made:?}"));
    }
    let shown = poold(&mut guest)?
        .disks()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|d| names.contains(&d.name))
        .map(|d| d.what)
        .collect::<Vec<_>>();
    if !shown.iter().all(|w| w.starts_with("POOL1 unit")) {
        return Err(format!("the disks are shown as {shown:?}"));
    }
    power_off(&mut guest, Duration::from_secs(60))?;
    drop(guest);

    // Again: assembled from the labels.
    let mut guest = host(run, &disks)?;
    guest.seen(&posted(&refcode::POOL_ASSEMBLED, "POOL1"), run.deadline())?;
    pool_until(&mut guest, "POOL1", run.deadline(), active)?;
    let kept = channel(&mut guest)?.text("/data/state/pools.toml")?;
    if !kept.contains("POOL1") {
        return Err(format!("pools.toml is {kept:?}"));
    }
    channel(&mut guest)?.ask("rm /data/state/pools.toml")?;
    power_off(&mut guest, Duration::from_secs(60))?;
    drop(guest);

    let mut guest = host(run, &disks)?;
    let again = pool_until(&mut guest, "POOL1", run.deadline(), active)?;
    let rebuilt = channel(&mut guest)?.text("/data/state/pools.toml")?;
    if !rebuilt.contains("POOL1") || !rebuilt.contains(&again.uuid) {
        return Err(format!("pools.toml was made again as {rebuilt:?}"));
    }
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "a mirrored pool of {} and {} made ({} bytes, Active, mounted at {}); assembled from its \
         labels at the next IPL with `{}` posted; its state file deleted and made again from \
         the labels at the IPL after",
        names[0],
        names[1],
        made.bytes,
        made.mount,
        refcode::POOL_ASSEMBLED.code
    ))
}

/// `pool-foreign`: a disk with what is not this host's on it, and a
/// pool disk of another host, are taken only with a typed confirmation.
pub fn foreign(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = pool_disks(run, 3, POOL_DISK_BYTES)?;
    // The first holds a GPT another system wrote; the second an xfs.
    write_foreign(&disks[0], b"EFI PART", 512)?;
    write_foreign(&disks[1], b"XFSB", 0)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, 3)?;
    let shown: Vec<_> = poold(&mut guest)?
        .disks()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|d| names.contains(&d.name))
        .collect();
    let what = |n: &str| {
        shown
            .iter()
            .find(|d| d.name == n)
            .map(|d| (d.what.clone(), d.foreign, d.free))
    };
    let (w0, f0, _) = what(&names[0]).ok_or("the first disk is not listed")?;
    let (w1, f1, _) = what(&names[1]).ok_or("the second disk is not listed")?;
    let (_, f2, free2) = what(&names[2]).ok_or("the third disk is not listed")?;
    if !f0 || !w0.contains("GPT") || !f1 || !w1.contains("xfs") || f2 || !free2 {
        return Err(format!("the disks are shown as {shown:?}"));
    }
    // Without the confirmation nothing is taken, and every disk named
    // is said.
    let refused = poold(&mut guest)?
        .create_pool(
            "POOL1".into(),
            "raid5".into(),
            names.clone(),
            Vec::new(),
            Vec::new(),
        )
        .map_err(|e| e.to_string())?;
    let Err(messages) = refused else {
        return Err("a pool was made of foreign disks with no confirmation".into());
    };
    let said_foreign = messages
        .iter()
        .filter(|m| m.is(&qvsmsg::DISK_FOREIGN))
        .count();
    if said_foreign != 2 {
        return Err(format!("the refusal said {messages:?}"));
    }
    if poold(&mut guest)?
        .pools()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|p| p.name == "POOL1")
    {
        return Err("the refused pool is there".into());
    }
    // With it, the pool is made.
    said(poold(&mut guest)?.create_pool(
        "POOL1".into(),
        "raid5".into(),
        names.clone(),
        Vec::new(),
        names[..2].to_vec(),
    ))?;
    pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, |p| {
        p.state == state::ACTIVE || p.state == state::RESYNCHRONIZING
    })?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "{} shown as `{w0}` and {} as `{w1}`, both foreign, {} not configured; a RAID 5 pool \
         of the three refused with QVS3007 for each foreign disk and nothing made; made when the \
         two were confirmed",
        names[0], names[1], names[2]
    ))
}

/// Write `bytes` at `at` of a blank disk image: what another system
/// left on it.
fn write_foreign(disk: &PathBuf, bytes: &[u8], at: u64) -> Result<(), String> {
    use std::os::unix::fs::FileExt;
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(disk)
        .map_err(|e| e.to_string())?;
    f.write_all_at(bytes, at).map_err(|e| e.to_string())
}

/// Pull unit `pull` (from 0) of a pool of `n` disks of `protection`,
/// with a hot spare when `spare`; what was seen.
fn pulled(run: &Run<'_>, protection: &str, n: usize, pulls: &[usize]) -> Result<String, String> {
    let disks = pool_disks(run, n + 1, POOL_DISK_BYTES)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, n + 1)?;
    make(&mut guest, "POOL1", protection, &names[..n], &names[n..])?;
    pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, active)?;
    said(poold(&mut guest)?.create_vdisk("POOL1".into(), "DATA".into(), 64, true))?;
    // A pattern written through the pool, to be read back after.
    let file = "/pools/POOL1/vdisks/DATA.img";
    // Parity writes three levels deep, and a unit's failure noticed by
    // the set, each take longer than an answer is usually given.
    let patience = Duration::from_mins(2);
    channel(&mut guest)?.ask_within(
        &format!("copy /dev/urandom 0 {file} 0 {}", 32 << 20),
        patience,
    )?;
    let (len, sum) = channel(&mut guest)?.sum(file)?;
    for unit in pulls {
        // QEMU's drive is taken from under the host: every transfer of
        // it fails from now on.
        guest.monitor(&format!("drive_del hd{}", unit + 1))?;
        // A transfer makes the set see it.
        channel(&mut guest)?.ask_within(
            &format!("copy {file} 0 {file}.again 0 {}", 1 << 20),
            patience,
        )?;
    }
    guest.seen(&posted(&refcode::POOL_UNIT_FAILED, "POOL1"), run.deadline())?;
    guest.seen(&posted(&refcode::POOL_SPARE_TAKEN, "POOL1"), run.deadline())?;
    let rebuilt = pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, |p| {
        p.state == state::ACTIVE || p.state == state::DEGRADED
    })?;
    guest.seen(
        &posted(&refcode::POOL_REBUILD_FINISHED, "POOL1"),
        run.deadline(),
    )?;
    let back = channel(&mut guest)?.sum(file)?;
    if back != (len, sum) {
        return Err(format!("the data is {back:?}, written as {:?}", (len, sum)));
    }
    let after = pool_until(&mut guest, "POOL1", run.deadline(), |_| true)?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "{protection} pool of {n} with a hot spare: unit(s) {pulls:?} pulled while it ran, \
         `{}` posted naming the unit, the hot spare taken and rebuilt to {}, then {}; 32 MiB \
         written before read back the same",
        refcode::POOL_UNIT_FAILED.code,
        rebuilt.state,
        after.state
    ))
}

/// `pool-pulled-mirrored`.
pub fn pulled_mirrored(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    pulled(run, "mirrored", 2, &[0])
}

/// `pool-pulled-raid5`.
pub fn pulled_raid5(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    pulled(run, "raid5", 3, &[1])
}

/// `pool-pulled-raid6`: two pulled.
pub fn pulled_raid6(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    pulled(run, "raid6", 4, &[0, 2])
}

/// `pool-lost`: more units lost than the protection covers.
pub fn lost(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = pool_disks(run, 3, POOL_DISK_BYTES)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, 3)?;
    make(&mut guest, "LOST", "mirrored", &names[..2], &[])?;
    make(&mut guest, "KEPT", "none", &names[2..], &[])?;
    pool_until(&mut guest, "LOST", Instant::now() + WORK_WITHIN, active)?;
    guest.monitor("drive_del hd1")?;
    guest.monitor("drive_del hd2")?;
    let _ = channel(&mut guest)?.ask("copy /dev/urandom 0 /pools/LOST/x 0 1048576");
    guest.seen(&posted(&refcode::POOL_FAILED, "LOST"), run.deadline())?;
    let lost = pool_until(&mut guest, "LOST", run.deadline(), |p| {
        p.state == state::FAILED
    })?;
    let kept = pool_until(&mut guest, "KEPT", run.deadline(), active)?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "both units of a mirrored pool pulled: `{}` posted and the pool {} ({}); the other pool {}",
        refcode::POOL_FAILED.code,
        lost.state,
        lost.failure,
        kept.state
    ))
}

/// `pool-power-pull`: the host's power pulled while a pool is written,
/// again and again; each time the pool is assembled and what was kept
/// is whole.
pub fn power_pull(run: &Run<'_>) -> Result<String, String> {
    const CUTS: u32 = 20;
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = pool_disks(run, 2, POOL_DISK_BYTES)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, 2)?;
    make(&mut guest, "POOL1", "mirrored", &names, &[])?;
    pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, active)?;
    power_off(&mut guest, Duration::from_secs(60))?;
    drop(guest);
    let mut copied = Vec::new();
    for cut in 0..CUTS {
        let mut guest = host(run, &disks)?;
        let at = |e: String| format!("after {cut} cuts: {e}");
        pool_until(&mut guest, "POOL1", run.deadline(), |p| !p.mount.is_empty()).map_err(at)?;
        if cut > 0 {
            let line = guest
                .line_with(
                    &posted(&refcode::POOL_RESYNC_STARTED, "POOL1"),
                    run.deadline(),
                )
                .unwrap_or_default();
            copied.push(line);
        }
        // Writes under way when the power goes.
        let _ = channel(&mut guest)?.ask(&format!(
            "copy /dev/urandom 0 /pools/POOL1/load{cut} 0 {}",
            8 << 20
        ));
        guest.kill();
    }
    let mut guest = host(run, &disks)?;
    pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, active)?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "{CUTS} power cuts of a host writing to a mirrored pool: assembled each time, the \
         resynchronizations {copied:?}, Active at the end"
    ))
}

/// `pool-kill`: the pool service killed while a pool is written: the
/// device holds the transfers until the service is back, and nothing
/// is lost.
pub fn kill(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = pool_disks(run, 2, POOL_DISK_BYTES)?;
    let mut guest = host(run, &disks)?;
    let names = disk_names(&mut guest, 2)?;
    make(&mut guest, "POOL1", "mirrored", &names, &[])?;
    pool_until(&mut guest, "POOL1", Instant::now() + WORK_WITHIN, active)?;
    let file = "/pools/POOL1/kept";
    channel(&mut guest)?.ask(&format!("copy /dev/urandom 0 {file} 0 {}", 16 << 20))?;
    let before = channel(&mut guest)?.sum(file)?;
    for _ in 0..3 {
        channel(&mut guest)?.ask("kill poold")?;
        std::thread::sleep(Duration::from_millis(300));
        pool_until(&mut guest, "POOL1", run.deadline(), |p| !p.mount.is_empty())?;
    }
    let after = channel(&mut guest)?.sum(file)?;
    if after != before {
        return Err(format!("the file is {after:?}, was {before:?}"));
    }
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(
        "the pool service killed three times: started again each time, the pool served on \
        and a file written before read back the same"
            .into(),
    )
}
