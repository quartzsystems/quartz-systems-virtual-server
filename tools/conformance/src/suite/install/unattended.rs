// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 3: an unattended installation with mirrored protection on two
//! blank disks, from the ISO as a USB stick; the disks read from the
//! host's side; then the installed host's first IPL.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hostdisk::record::HostRecord;
use hostdisk::set::MemberState;
use release::fact;

use super::{
    HOST, Kind, OnDisk, SERIALS, answers, blank_disks, boot, install_unattended, installed,
    machine, on_disk, power_off, protected,
};
use crate::qemu::Launch;
use crate::suite::base::{Run, STATE_DIR, all_ready, channel, fact_of, posted};

/// What the first synchronization of a pair is given.
const SYNC_WITHIN: Duration = Duration::from_secs(180);

fn check_disk(found: &OnDisk, ukis: &(Vec<u8>, Vec<u8>), which: usize) -> Result<(), String> {
    if found.slot_a != ukis.0 || found.slot_b != ukis.1 {
        return Err(format!(
            "disk {which}: the ESP's UKIs are not the build's ({} and {} bytes)",
            found.slot_a.len(),
            found.slot_b.len()
        ));
    }
    if found.record.0 != HostRecord::INITIAL {
        return Err(format!(
            "disk {which}: the A/B record is {:?}",
            found.record
        ));
    }
    Ok(())
}

/// Suite 3.
pub fn mirrored(run: &Run<'_>) -> Result<String, String> {
    let disks = blank_disks(run, "mirror", 2)?;
    let paths: Vec<&Path> = disks.iter().map(PathBuf::as_path).collect();
    install_unattended(run, &paths, &answers(&SERIALS, "", ""), true)?;

    // Both disks hold both UKIs, a valid record and the same label.
    let (a, b) = run
        .images
        .test_ukis
        .as_ref()
        .ok_or("the test image's UKIs are not built")?;
    let ukis = (
        fs::read(a).map_err(|e| e.to_string())?,
        fs::read(b).map_err(|e| e.to_string())?,
    );
    let found = [on_disk(paths[0])?, on_disk(paths[1])?];
    for (i, disk) in found.iter().enumerate() {
        check_disk(disk, &ukis, i)?;
    }
    let label = found[0].label.clone().ok_or("disk 0 has no set label")?;
    if found[1].label.as_ref() != Some(&label) {
        return Err("the two disks' set labels differ".into());
    }
    let members: Vec<_> = label
        .members
        .iter()
        .map(|m| (m.partition, m.state))
        .collect();
    let wanted = [
        (found[0].data_guid, MemberState::Active),
        (found[1].data_guid, MemberState::Resuming),
    ];
    if members != wanted || !label.clean {
        return Err(format!("the set label is not as installed: {label:?}"));
    }

    // The first IPL: /data on the set, the second unit written.
    let places: Vec<Option<&Path>> = paths.iter().map(|d| Some(*d)).collect();
    let mut guest = boot(run, &machine(run, &places))?;
    let deadline = run.deadline();
    guest.seen(&posted(&refcode::SET_ASSEMBLED, ""), deadline)?;
    let status = channel(&mut guest)?.status()?;
    let data = fact_of(&status, fact::DATA)?.to_string();
    if !data.starts_with("/dev/ublkb") {
        return Err(format!("/data is on {data}, not on the protected set"));
    }
    if fact_of(&status, fact::HOST)? != HOST {
        return Err(format!("the host is {}", fact_of(&status, fact::HOST)?));
    }
    let storage = protected(&mut guest, SYNC_WITHIN)?;
    guest.seen(&posted(&refcode::RESYNC_FINISHED, ""), run.deadline())?;
    let status = channel(&mut guest)?.status()?;
    all_ready(&status)?;
    let state = channel(&mut guest)?.ls(STATE_DIR)?;
    // What the installer wrote, and the identity this first IPL made.
    for file in [
        "host.toml",
        "network.toml",
        users::FILE,
        "identity.key",
        "identity.crt",
    ] {
        if !state.iter().any(|f| f == file) {
            return Err(format!("{STATE_DIR} has no {file}: {state:?}"));
        }
    }
    guest.seen(&posted(&refcode::IDENTITY_GENERATED, ""), run.deadline())?;
    let certificate = channel(&mut guest)?.text(&format!("{STATE_DIR}/identity.crt"))?;
    if !certificate.starts_with("-----BEGIN CERTIFICATE-----") {
        return Err("identity.crt is not a PEM certificate".into());
    }
    power_off(&mut guest)?;
    drop(guest);

    // Stopped in order: both units active, the label clean, on both.
    for (i, disk) in paths.iter().enumerate() {
        let label = on_disk(disk)?
            .label
            .ok_or_else(|| format!("disk {i} lost its label"))?;
        if !label.clean || !label.protected() {
            return Err(format!(
                "disk {i}: after the power off the label is {label:?}"
            ));
        }
    }
    let units: Vec<String> = storage
        .units
        .iter()
        .map(|u| format!("unit {} at {} {}", u.number, u.location, u.state))
        .collect();
    Ok(format!(
        "installed from the ISO as a USB stick with no key pressed; both disks hold both UKIs \
         ({} and {} bytes), record sequence {} and the set label; first IPL: /data on {data}, {}, \
         the host's identity made; powered off clean",
        ukis.0.len(),
        ukis.1.len(),
        found[0].record.1,
        units.join(", ")
    ))
}

/// The release image's ISO.
fn release_iso<'a>(run: &Run<'a>) -> Result<&'a Path, String> {
    run.images
        .iso
        .as_deref()
        .filter(|p| p.is_file())
        .ok_or_else(|| "the release ISO is not built; run `all`".into())
}

/// Phase 0's suite on a disk the release installer wrote: an IPL from
/// slot A, then from slot B by a firmware boot entry.
fn phase0(run: &Run<'_>) -> Result<String, String> {
    let disk = run.file("release.img");
    crate::qemu::image::blank(&disk, super::DISK_BYTES)?;
    super::install_from(
        run,
        release_iso(run)?,
        &[disk.as_path()],
        &answers(&SERIALS[..1], "", ""),
        false,
    )?;
    let allow = crate::suite::allow_no_kvm();
    let mut seen = Vec::new();
    // The release image has no test hooks: no channel, the console only.
    let base = Launch {
        disk: Some(disk.clone()),
        snapshot: true,
        ..run.base.clone()
    };
    let vars = crate::suite::slotb::vars(&disk, &run.base.firmware, run.work)
        .map_err(|e| e.to_string())?;
    for (source, vars, heartbeats) in [
        ("IPL slot A (qsvs.slot=A)", None, true),
        ("IPL slot B (qsvs.slot=B)", Some(vars), false),
    ] {
        let mut guest = run.boot(&Launch {
            vars,
            ..base.clone()
        })?;
        let (detail, _) =
            crate::suite::ipl_check(&mut guest, source, heartbeats, run.deadline(), allow)?;
        seen.push(detail);
    }
    Ok(seen.join("; "))
}

/// Suite 13.
pub fn installed_base(run: &Run<'_>) -> Result<String, String> {
    let ipl = phase0(run)?;
    // Phase 1's ten, on a disk the test image's installer wrote.
    let disks = installed(run, Kind::Single)?;
    let on_installed = Run {
        disk: disks[0].as_path(),
        installed: true,
        ..*run
    };
    let mut passed = Vec::new();
    for (name, suite) in crate::suite::base::SUITES {
        suite(&on_installed).map_err(|e| format!("`{name}` on the installed system: {e}"))?;
        passed.push(*name);
    }
    Ok(format!(
        "on a disk the release installer wrote: {ipl}; on a disk the test image's installer \
         wrote, the {} suites of the base system pass: {}",
        passed.len(),
        passed.join(", ")
    ))
}
