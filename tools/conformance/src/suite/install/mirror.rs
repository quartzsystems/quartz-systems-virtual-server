// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 4, 5 and 8: a mirrored host with either disk removed, a
//! missing unit replaced from Service Tools and rebuilt, and the
//! storage service killed under write load.

use std::path::Path;
use std::time::{Duration, Instant};

use qsvshook::proto::command;
use release::fact;

use super::{
    DISK_BYTES, Kind, PASSWORD, USER, boot, golden, installed, machine, on_disk, option, power_off,
    protected, sign_on,
};
use crate::qemu::term::key;
use crate::qemu::{Drive, image};
use crate::suite::base::{Guest, Run, all_ready, channel, fact_of};

/// The title of the disk units display.
const DISK_UNITS: &str = "Work with Disk Units";
/// What a rebuild is given.
const REBUILD_WITHIN: Duration = Duration::from_secs(240);
/// A file the suites leave on `/data` and look for afterwards.
const MARKER: &str = "/data/state/conformance-marker";
/// What the marker holds.
const MARKED: &str = "written before the disk went";

fn put_marker(guest: &mut Guest<'_>) -> Result<(), String> {
    channel(guest)?
        .ask(&format!(
            "{} {MARKER} {}",
            command::PUT,
            hex::encode(MARKED)
        ))
        .map(|_| ())
}

fn check_marker(guest: &mut Guest<'_>) -> Result<(), String> {
    let found = channel(guest)?.text(MARKER)?;
    if found == MARKED {
        Ok(())
    } else {
        Err(format!("{MARKER} holds {found:?}"))
    }
}

/// `/data` is on the protected set.
fn served(guest: &mut Guest<'_>) -> Result<String, String> {
    let status = channel(guest)?.status()?;
    let data = fact_of(&status, fact::DATA)?.to_string();
    if data.starts_with("/dev/ublkb") {
        Ok(data)
    } else {
        Err(format!("/data is on {data}, not on the protected set"))
    }
}

/// Suite 4.
pub fn degraded(run: &Run<'_>) -> Result<String, String> {
    let mut seen = Vec::new();
    // Unit 2's disk removed, then unit 1's: each time from a pair in step.
    for missing in [1usize, 0] {
        let disks = installed(run, Kind::Mirrored)?;
        let place = |i: usize| (i != missing).then_some(disks[i].as_path());
        let mut launch = machine(run, &[place(0), place(1)]);
        launch.keys = true;
        let mut guest = boot(run, &launch)?;
        let deadline = run.deadline();
        let data = served(&mut guest)?;
        let status = channel(&mut guest)?.status()?;
        all_ready(&status)?;
        let storage = channel(&mut guest)?.storage()?;
        let unit = &storage.units[missing];
        let other = &storage.units[1 - missing];
        if unit.state != "Missing" || other.state != "Active" || unit.location.is_empty() {
            return Err(format!(
                "with disk {missing} removed the units are {:?}",
                storage.units
            ));
        }
        // The attention code names the unit by its location code.
        let wanted = format!(
            "{}: unit {} at {}",
            refcode::UNIT_FAILED.line(),
            unit.number,
            unit.location
        );
        guest.seen(&wanted, deadline)?;
        // The host runs: it writes, and reads back.
        put_marker(&mut guest)?;
        check_marker(&mut guest)?;

        // Service Tools shows which unit, by location code.
        sign_on(&mut guest, USER, PASSWORD, deadline)?;
        let shown = option(&mut guest, 2, DISK_UNITS, deadline)?;
        let row = shown
            .lines()
            .find(|l| l.contains(&unit.location))
            .unwrap_or_default();
        if !row.trim_end().ends_with("Missing") || !shown.contains("NOT protected now") {
            return Err(format!("the disk units display does not show it:\n{shown}"));
        }
        if missing == 1 {
            golden("tools-disks-degraded", &shown)?;
        }
        guest.type_keys(key::F3)?;
        guest.type_keys(key::F3)?;
        power_off(&mut guest)?;
        seen.push(format!(
            "without unit {} the host IPLs on {data}, posts `{wanted}` and shows the unit Missing",
            unit.number
        ));
    }
    Ok(seen.join("; "))
}

/// Suite 5.
pub fn replace(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Mirrored)?;
    let blank = run.file("replacement.img");
    image::blank(&blank, DISK_BYTES)?;
    let new = Drive {
        path: blank.clone(),
        serial: "QSVSNEW".into(),
        place: Some(2),
    };

    // Unit 2's disk is gone; a blank disk is in another place.
    let mut launch = machine(run, &[Some(disks[0].as_path()), None]);
    launch.extra.push(new.clone());
    launch.keys = true;
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();
    put_marker(&mut guest)?;
    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    let units = option(&mut guest, 2, DISK_UNITS, deadline)?;
    golden("tools-disks-missing", &units)?;
    guest.type_keys(&format!("2{}", key::ENTER))?;
    let select = guest.screen("the disks to choose from", deadline, |grid| {
        grid.contains("to put in the place of unit 2")
    })?;
    golden("tools-disk-select", &select)?;
    guest.type_keys(&format!("1{}", key::ENTER))?;
    let confirm = super::titled(&mut guest, "Confirm Erase of Disk", deadline)?;
    golden("tools-disk-confirm", &confirm)?;
    guest.type_keys(key::F10)?;
    // The rebuild, watched to its end.
    let rebuilt = guest.screen(
        "the rebuild's end",
        Instant::now() + REBUILD_WITHIN,
        |grid| grid.contains("Both units are in step."),
    )?;
    golden("tools-disk-rebuilt", &rebuilt)?;
    for code in [&refcode::REBUILD_STARTED, &refcode::REBUILD_FINISHED] {
        super::in_history(&mut guest, code, "unit 2", run.deadline())?;
    }
    let storage = protected(&mut guest, Duration::from_secs(30))?;
    if storage.units[1].serial != new.serial {
        return Err(format!("unit 2 is {:?}", storage.units[1]));
    }
    guest.type_keys(key::ENTER)?;
    power_off(&mut guest)?;
    drop(guest);
    let label = on_disk(&blank)?
        .label
        .ok_or("the new disk has no set label")?;
    if !label.clean || !label.protected() {
        return Err(format!("the new disk's label is {label:?}"));
    }

    // The original disk is removed: the host IPLs from the new one.
    let mut launch = machine(run, &[None, None]);
    launch.extra.push(new);
    let mut guest = boot(run, &launch)?;
    let data = served(&mut guest)?;
    check_marker(&mut guest)?;
    let storage = channel(&mut guest)?.storage()?;
    let states: Vec<&str> = storage.units.iter().map(|u| u.state.as_str()).collect();
    if states != ["Missing", "Active"] {
        return Err(format!("on the new disk alone the units are {states:?}"));
    }
    power_off(&mut guest)?;
    Ok(format!(
        "unit 2 missing and a blank disk put in its place from Service Tools option 2: prepared, \
         rebuilt to `Both units are in step`, `{}` and `{}` posted; with the original disk \
         removed the host IPLs from the new one on {data} and reads what was written before",
        refcode::REBUILD_STARTED.line(),
        refcode::REBUILD_FINISHED.line()
    ))
}

/// Beyond the thirteen: mirrored protection started on an unprotected
/// load source from Service Tools, with a second disk. It takes effect
/// at the next IPL, which assembles the set and writes the new unit.
pub fn start(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Single)?;
    let second = run.file("second.img");
    image::blank(&second, DISK_BYTES)?;
    let both = [Some(disks[0].as_path()), Some(second.as_path())];
    let mut launch = machine(run, &both);
    launch.keys = true;
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();
    put_marker(&mut guest)?;
    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    let units = option(&mut guest, 2, DISK_UNITS, deadline)?;
    golden("tools-disks-unprotected", &units)?;
    guest.type_keys(&format!("1{}", key::ENTER))?;
    guest.screen("the disks to choose from", deadline, |grid| {
        grid.contains("to mirror the load source with")
    })?;
    guest.type_keys(&format!("1{}", key::ENTER))?;
    super::titled(&mut guest, "Confirm Erase of Disk", deadline)?;
    guest.type_keys(key::F10)?;
    let prepared = guest.screen(
        "the disk prepared",
        Instant::now() + REBUILD_WITHIN,
        |grid| grid.contains("Mirrored protection starts at the next IPL"),
    )?;
    golden("tools-disk-protect", &prepared)?;
    super::in_history(
        &mut guest,
        &refcode::PROTECTION_STARTED,
        "unit 2",
        run.deadline(),
    )?;
    // Until the IPL the host runs on the partition as before.
    let status = channel(&mut guest)?.status()?;
    let data = fact_of(&status, fact::DATA)?.to_string();
    if data.starts_with("/dev/ublkb") {
        return Err(format!("/data moved to {data} without an IPL"));
    }
    guest.type_keys(key::ENTER)?;
    power_off(&mut guest)?;
    drop(guest);

    // The next IPL: the set, with the new unit written from the first.
    let mut guest = boot(run, &machine(run, &both))?;
    let served = served(&mut guest)?;
    guest.seen(&posted_code(&refcode::SET_ASSEMBLED), run.deadline())?;
    let storage = protected(&mut guest, REBUILD_WITHIN)?;
    check_marker(&mut guest)?;
    power_off(&mut guest)?;
    drop(guest);
    for disk in [&disks[0], &second] {
        let label = on_disk(disk)?.label.ok_or("a disk has no set label")?;
        if !label.clean || !label.protected() {
            return Err(format!("after it all a label is {label:?}"));
        }
    }
    Ok(format!(
        "an unprotected load source on {data} given a second disk from Service Tools option 2: \
         prepared while the host ran, `{}` posted; at the next IPL /data is on {served}, unit 2 \
         was written from unit 1 and both are {}; what was on /data before is there",
        refcode::PROTECTION_STARTED.line(),
        storage.units[1].state
    ))
}

fn posted_code(code: &refcode::RefCode) -> String {
    crate::suite::base::posted(code, "")
}

/// The lines of the load's list: the files it wrote whole.
fn load_done(guest: &mut Guest<'_>) -> Result<Vec<(u64, u64)>, String> {
    let text = channel(guest)?.text("/data/load/done").unwrap_or_default();
    Ok(text
        .lines()
        .filter_map(|l| {
            let (n, megabytes) = l.split_once(' ')?;
            Some((n.parse().ok()?, megabytes.parse().ok()?))
        })
        .collect())
}

/// Suite 8.
pub fn storage_kill(run: &Run<'_>) -> Result<String, String> {
    /// How often the service is killed.
    const KILLS: usize = 3;
    /// Megabytes a file of the load.
    const FILE_MB: u64 = 4;
    let disks = installed(run, Kind::Mirrored)?;
    let places: Vec<Option<&Path>> = disks.iter().map(|d| Some(d.as_path())).collect();
    let mut guest = boot(run, &machine(run, &places))?;
    let data = served(&mut guest)?;
    put_marker(&mut guest)?;
    channel(&mut guest)?.ask(&format!("{} {FILE_MB}", command::LOAD))?;
    let mut pids = Vec::new();
    for kill in 0..KILLS {
        // The load is writing.
        let before = load_done(&mut guest)?.len();
        let deadline = Instant::now() + Duration::from_secs(30);
        while load_done(&mut guest)?.len() <= before {
            if Instant::now() > deadline {
                return Err(format!("before kill {kill} the load writes nothing"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let pid = channel(&mut guest)?
            .status()?
            .service(qsvsstgd::NAME)
            .map_or(0, |s| s.pid);
        channel(&mut guest)?.ask(&format!("{} {}", command::KILL, qsvsstgd::NAME))?;
        // PID 1 starts it again; the new server takes the device over.
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = channel(&mut guest)?.until("the storage service again", deadline, |s| {
            s.service(qsvsstgd::NAME)
                .is_some_and(|s| s.state == "ready" && s.pid != pid)
        })?;
        pids.push((pid, status.service(qsvsstgd::NAME).map_or(0, |s| s.pid)));
        // What waited goes on: the load writes more files whole.
        let before = load_done(&mut guest)?.len();
        while load_done(&mut guest)?.len() <= before {
            if Instant::now() > deadline {
                return Err(format!("after kill {kill} the load did not go on"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    channel(&mut guest)?.ask(&format!("{} 0", command::LOAD))?;
    std::thread::sleep(Duration::from_secs(2));

    // Nothing is lost: the marker, and every file the list says is whole
    // and that the load has not taken away again.
    check_marker(&mut guest)?;
    let done = load_done(&mut guest)?;
    let mut checked = 0;
    for (n, megabytes) in done.iter().rev().take(3) {
        let (len, sum) = channel(&mut guest)?.sum(&format!("/data/load/file-{n}"))?;
        let bytes = megabytes << 20;
        if (len, sum) != (bytes, bytes * (n % 251)) {
            return Err(format!("file-{n} is {len} bytes with sum {sum}"));
        }
        checked += 1;
    }
    for code in [
        &refcode::DATA_NOT_MOUNTED,
        &refcode::STORAGE_LOST,
        &refcode::UNIT_FAILED,
        &refcode::RESYNC_STARTED,
    ] {
        if guest.said(&code.tag()) {
            return Err(format!("the host posted {}", code.line()));
        }
    }
    let storage = protected(&mut guest, Duration::from_secs(30))?;
    let status = channel(&mut guest)?.status()?;
    all_ready(&status)?;
    let failures = status.service(qsvsstgd::NAME).map_or(0, |s| s.failures);
    power_off(&mut guest)?;
    drop(guest);
    for disk in &disks {
        let label = on_disk(disk)?.label.ok_or("a disk lost its set label")?;
        if !label.clean || !label.protected() {
            return Err(format!("after it all a label is {label:?}"));
        }
    }
    let restarts: Vec<String> = pids
        .iter()
        .map(|(old, new)| format!("process {old} to {new}"))
        .collect();
    Ok(format!(
        "the storage service killed {KILLS} times while {FILE_MB} MiB files were written to \
         {data}: started again each time ({}; {failures} failures counted), the writes went on, \
         {} files written and the last {checked} read back whole; both units still {}, no \
         resynchronization needed",
        restarts.join(", "),
        done.len(),
        storage.units[0].state
    ))
}
