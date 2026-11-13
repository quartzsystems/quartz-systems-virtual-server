// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `mirror-split`: each disk of a mirrored pair started without the
//! other, then both together. The host picks no winner: it posts its
//! code, mounts no `/data`, starts nothing and asks which unit to
//! keep. The unit chosen is the one the host then runs on, and the
//! other is written again from it.

use std::path::Path;
use std::time::{Duration, Instant};

use qsvshook::proto::command;
use release::fact;

use super::{
    Kind, boot, golden, in_history, installed, machine, on_disk, power_off, protected, titled,
};
use crate::qemu::term::key;
use crate::suite::base::{Guest, Run, base_ready, channel, fact_of, posted};

/// The title of the choice.
const KEEP: &str = "Select Load-Source Unit to Keep";
/// The title of its confirmation.
const CONFIRM: &str = "Confirm Unit to Keep";
/// What the copy of the other unit is given.
const COPY_WITHIN: Duration = Duration::from_secs(240);
/// How long the host is watched doing nothing before the choice.
const WATCHED: Duration = Duration::from_secs(8);
/// Where each unit leaves a file while it runs alone.
const DIR: &str = "/data/state";

fn name(unit: usize) -> String {
    format!("conformance-alone-{unit}")
}

fn put(guest: &mut Guest<'_>, unit: usize) -> Result<(), String> {
    channel(guest)?
        .ask(&format!(
            "{} {DIR}/{} {}",
            command::PUT,
            name(unit),
            hex::encode(format!("written while unit {unit} ran alone"))
        ))
        .map(|_| ())
}

/// Which units' files are on `/data`.
fn files(guest: &mut Guest<'_>) -> Result<Vec<usize>, String> {
    let found = channel(guest)?.ls(DIR)?;
    Ok([1, 2]
        .into_iter()
        .filter(|unit| found.iter().any(|f| f.contains(&name(*unit))))
        .collect())
}

/// Watch the host for a while before a unit is chosen: it mounts no
/// `/data` and starts no service.
fn nothing_started(guest: &mut Guest<'_>) -> Result<(), String> {
    let watched = Instant::now() + WATCHED;
    while Instant::now() < watched {
        guest.drain();
        std::thread::sleep(Duration::from_millis(250));
    }
    for code in [
        &refcode::DATA_MOUNTED,
        &refcode::DATA_NOT_MOUNTED,
        &refcode::SET_ASSEMBLED,
        &refcode::SERVICES_STARTED,
        &refcode::BASE_READY,
    ] {
        if guest.said(&code.tag()) {
            return Err(format!(
                "before a unit was chosen the host posted {}",
                code.line()
            ));
        }
    }
    Ok(())
}

/// The suite.
pub fn split(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Mirrored)?;
    let both = [Some(disks[0].as_path()), Some(disks[1].as_path())];

    // Each disk in turn without the other: each goes on alone.
    for unit in [1usize, 2] {
        let place = |i: usize| (i + 1 == unit).then_some(disks[i].as_path());
        let mut guest = boot(run, &machine(run, &[place(0), place(1)]))?;
        put(&mut guest, unit)?;
        if files(&mut guest)? != [unit] {
            return Err(format!("unit {unit} alone does not hold its own file only"));
        }
        power_off(&mut guest)?;
    }
    let labels: Vec<_> = disks
        .iter()
        .map(|d| on_disk(d).map(|found| found.label))
        .collect::<Result<_, _>>()?;
    let [Some(one), Some(two)] = labels.as_slice() else {
        return Err("a disk has no set label".into());
    };
    if one.written == 0 || two.written < one.written {
        return Err(format!(
            "the labels were written at {} and {}",
            one.written, two.written
        ));
    }

    // Both together: the code, the choice, and nothing else.
    let mut launch = machine(run, &both);
    launch.keys = true;
    let mut guest = run.boot(&launch)?;
    let deadline = run.deadline();
    guest.seen(&posted(&refcode::UNITS_APART, "unit 1 at "), deadline)?;
    let asked = titled(&mut guest, KEEP, deadline)?;
    golden("keep-unit", &asked)?;
    for unit in [one, two] {
        for member in &unit.members {
            if !asked.contains(&member.location) || !asked.contains(&member.serial) {
                return Err(format!(
                    "the choice does not name the unit at {}:\n{asked}",
                    member.location
                ));
            }
        }
    }
    nothing_started(&mut guest)?;
    // The panel is left by a choice only.
    guest.type_keys(key::F3)?;
    guest.screen("the refusal to leave", deadline, |grid| {
        grid.contains("must be chosen")
    })?;
    // Unit 2 is kept: the one written last.
    guest.type_keys(&format!("{}1{}", key::TAB, key::ENTER))?;
    let confirm = titled(&mut guest, CONFIRM, deadline)?;
    golden("keep-unit-confirm", &confirm)?;
    guest.type_keys(key::F10)?;

    // The IPL goes on from unit 2, and unit 1 is written again.
    base_ready(&mut guest, deadline)?;
    guest.channel(deadline)?;
    // The console had the serial port when these were posted.
    in_history(&mut guest, &refcode::UNITS_APART, "unit 1 at ", deadline)?;
    in_history(&mut guest, &refcode::UNIT_KEPT, "unit 2 at ", deadline)?;
    in_history(&mut guest, &refcode::SET_ASSEMBLED, "", deadline)?;
    let status = channel(&mut guest)?.status()?;
    let data = fact_of(&status, fact::DATA)?.to_string();
    if !data.starts_with("/dev/ublkb") {
        return Err(format!("/data is on {data}, not on the protected set"));
    }
    if files(&mut guest)? != [2] {
        return Err("after unit 2 was kept /data does not hold its file only".into());
    }
    in_history(&mut guest, &refcode::RESYNC_STARTED, "unit 1", deadline)?;
    let storage = protected(&mut guest, COPY_WITHIN)?;
    guest.seen(
        &posted(&refcode::RESYNC_FINISHED, "unit 1"),
        Instant::now() + Duration::from_secs(30),
    )?;
    power_off(&mut guest)?;
    drop(guest);
    for disk in &disks {
        let label = on_disk(disk)?.label.ok_or("a disk lost its set label")?;
        if !label.clean || !label.protected() {
            return Err(format!("after it all a label is {label:?}"));
        }
    }

    // Unit 1's disk alone now holds what unit 2 had.
    let alone: [Option<&Path>; 2] = [Some(disks[0].as_path()), None];
    let mut guest = boot(run, &machine(run, &alone))?;
    let held = files(&mut guest)?;
    power_off(&mut guest)?;
    if held != [2] {
        return Err(format!(
            "unit 1's disk holds the files of units {held:?} after it was written from unit 2"
        ));
    }
    Ok(format!(
        "each disk run without the other, then both: `{}` posted, /data not mounted and no \
         service started for {} s, the choice shown with both units and left by a choice only; \
         unit 2 kept, `{}` posted, the IPL went on from it on {data}, unit 1 was written again \
         and both are {}; unit 1's disk alone then holds what unit 2 had",
        refcode::UNITS_APART.line(),
        WATCHED.as_secs(),
        refcode::UNIT_KEPT.line(),
        storage.units[0].state
    ))
}
