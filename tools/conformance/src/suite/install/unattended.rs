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
    HOST, OnDisk, SERIALS, answers, blank_disks, boot, install_unattended, machine, on_disk,
    power_off, protected,
};
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
    for file in ["host.toml", "network.toml", users::FILE] {
        if !state.iter().any(|f| f == file) {
            return Err(format!("{STATE_DIR} has no {file}: {state:?}"));
        }
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
         ({} and {} bytes), record sequence {} and the set label; first IPL: /data on {data}, {}; \
         powered off clean",
        ukis.0.len(),
        ukis.1.len(),
        found[0].record.1,
        units.join(", ")
    ))
}

/// Suite 13.
pub fn installed_base(_run: &Run<'_>) -> Result<String, String> {
    Err("not written yet".into())
}
