// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 13: tape pass-through against the double, a tape drive that
//! the test image's `scsi_debug` emulates on the host. The partition's
//! system is the test image's copy, whose probe uses the drive through
//! `st` and `sg` and says what it saw (`lic/qsvsinit`'s hooks).

use std::time::{Duration, Instant};

use model::profile::MediaDevice;
use partd::Calls as _;
use qsvshook::proto::command;

use super::devices::{end, probed, profile};
use crate::qemu::session::Session;
use crate::suite::base::{Guest, Run, channel, posted};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::partition::{fresh_host, host, launch, unit};

/// `scsi_debug`'s controls.
const DOUBLE: &str = "/sys/bus/pseudo/drivers/scsi_debug";
/// The tape device's slot.
const SLOT: u32 = 3;
/// What the double says it is.
const VENDOR: &str = "Linux";
const PRODUCT: &str = "scsi_debug";

/// A host with the test image's copy as a unit and the double made.
fn rig<'a>(run: &Run<'a>) -> Result<Guest<'a>, String> {
    fresh_host(run)?;
    let units = vec![unit(run, run.disk)?];
    let l = launch(run, &units)?;
    let mut guest = host(run, &l)?;
    // A sequential-access device, then a host of it.
    for (file, value) in [("ptype", "1"), ("add_host", "1")] {
        channel(&mut guest)?.ask(&format!("{} {DOUBLE}/{file} {value}", command::WRITE))?;
    }
    Ok(guest)
}

/// The double as the partition manager lists it, once it is there.
fn drive(g: &mut Session, deadline: Instant) -> Result<partd::proto::TapeDrive, String> {
    loop {
        let drives = partd(g)?.tape_drives().map_err(|e| e.to_string())?;
        if let Some(d) = drives.iter().find(|d| d.present && d.product == PRODUCT) {
            return Ok(d.clone());
        }
        if Instant::now() >= deadline {
            return Err(format!("the double is not listed: {drives:?}"));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// A profile of the test image's copy with the drive in its tape slot.
fn with_drive(name: &str, location: &str) -> model::profile::Profile {
    let mut p = profile(name, &[]);
    p.tape = vec![MediaDevice {
        slot: SLOT,
        media: Some(format!("{}{location}", model::validate::DRIVE_MARK)),
    }];
    p
}

/// Suite 13.
pub fn double(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run)?;
    let g = &mut guest;
    let d = drive(g, run.deadline())?;
    let location = d.location.clone();
    said(partd(g)?.release(location.clone(), location.clone()))?;
    let first = manager::partition(g, "FIRST")?;
    let second = manager::partition(g, "SECOND")?;
    let p = with_drive("TAPE", &location);
    said(partd(g)?.put_profile(first, p.clone()))?;
    said(partd(g)?.activate(first, "TAPE".into(), None, None))?;
    // What the drive said of itself, through the guest's own kernel.
    let seen = probed(g, "FIRST", &format!("TAPE st0 {VENDOR}"), run)?;
    if !seen.contains(VENDOR) || !seen.contains(PRODUCT) {
        return Err(format!("the guest's drive: {seen}"));
    }
    let data = probed(g, "FIRST", "TAPE st0 write", run)?;
    if !data.contains("read back ok") {
        return Err(format!("written and read back: {data}"));
    }
    // Refused before the drive: ILLEGAL REQUEST, INVALID COMMAND.
    let firmware = probed(g, "FIRST", "TAPE st0 WRITE BUFFER", run)?;
    if !firmware.ends_with("status 0x2 sense 0x5/0x20") {
        return Err(format!("WRITE BUFFER: {firmware}"));
    }
    // Had it reached the double, the next command would find its unit
    // attention, microcode changed (3F).
    let tur = g.line_with("FIRST| PIO TAPE st0 TUR", run.deadline())?;
    if !tur.contains("status 0x0 sense 0x0/0x0") {
        return Err(format!("the first TEST UNIT READY after it: {tur}"));
    }
    // One holder at a time.
    said(partd(g)?.put_profile(second, with_drive("TAPE", &location)))?;
    let refused = partd(g)?
        .activate(second, "TAPE".into(), None, None)
        .map_err(|e| e.to_string())?
        .unwrap_err();
    if !refused[0].is(&qvsmsg::DEVICE_HELD) || !refused[0].text.contains("FIRST") {
        return Err(format!("a second holder: {refused:?}"));
    }
    // The drive goes away under the running partition.
    let st = channel(g)?.ls("/sys/class/scsi_tape")?;
    let host_st = st
        .iter()
        .find(|n| n.starts_with("st") && n[2..].bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| format!("no tape device on the host: {st:?}"))?
        .clone();
    channel(g)?.ask(&format!(
        "{} /sys/class/scsi_tape/{host_st}/device/delete 1",
        command::WRITE
    ))?;
    // Whole, even with a kernel message written into its line.
    let gone = posted(&refcode::DRIVE_GONE, &location);
    g.seen(&gone, run.deadline())?;
    // The probe says a TEST UNIT READY's answer only when it changes.
    let deadline = run.deadline();
    let after = loop {
        g.drain();
        let last = g.line_with("FIRST| PIO TAPE st0 TUR", deadline)?;
        if last != tur {
            break last;
        }
        if Instant::now() >= deadline {
            return Err(format!("no command failed after the drive went: {last}"));
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    if after.contains("status 0x0 sense 0x0/0x0") {
        return Err(format!("a command after the drive went: {after}"));
    }
    super::host::problem_of(g, &refcode::DRIVE_GONE)?;
    end(g, first)?;
    Ok(format!(
        "{location} ({VENDOR} {PRODUCT}) released and given to FIRST: `{seen}`, `{data}`; \
         WRITE BUFFER refused in front of the drive (`{firmware}`) and the drive never saw \
         it (`{}`); SECOND refused while FIRST held it: `{}`; the drive deleted: `{}`, \
         FIRST's next command `{}`, and its problem open",
        tur.trim(),
        refused[0],
        gone.trim(),
        after.trim()
    ))
}
