// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Definitions and profiles: three partitions are created with
//! profiles, each definition and profile survives a host restart, and
//! validation rejects a bad profile with every error listed.

use std::time::Instant;

use model::Keylock;
use model::profile::Sharing;
use partd::{Calls as _, State};
use qsvshook::proto::command;

use super::manager::{self, partd, said};
use super::resources::{spin, spinner_launch, spinners};
use super::{host, host_unaffected};
use crate::suite::base::{Run, channel, posted};

/// The suite.
#[allow(clippy::too_many_lines)] // one suite, told in order
pub fn define(run: &Run<'_>) -> Result<String, String> {
    let l = spinner_launch(run, 1)?;
    let (mut guest, ids) = spinners(run, &l, 3)?;
    if ids != [1, 2, 3] {
        return Err(format!("the first three partitions have the IDs {ids:?}"));
    }
    // A dedicated profile and a shared one for each; the second
    // partition's definition changed in everything that can be.
    let mut kept = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let mut dedicated = spin(0, 1);
        dedicated.name = "NORMAL".into();
        let mut shared = spin(0, 2).shared(50, Sharing::Uncapped);
        shared.name = "SHARED".into();
        shared.weight = Some(64 + 10 * u32::try_from(i).unwrap_or(0));
        manager::put(&mut guest, *id, &dedicated)?;
        manager::put(&mut guest, *id, &shared)?;
    }
    let mut changed = said(partd(&mut guest)?.definition(ids[1]))?;
    changed.name = "RENAMED".into();
    changed.keylock = Keylock::Manual;
    changed.default_profile = "SHARED".into();
    changed.auto_start = model::AutoStart {
        enabled: false,
        order: 7,
        delay_secs: 30,
    };
    said(partd(&mut guest)?.change(changed.clone()))?;
    for id in &ids {
        let definition = said(partd(&mut guest)?.definition(*id))?;
        let names = said(partd(&mut guest)?.profiles(*id))?;
        let mut profiles = Vec::new();
        for name in &names {
            profiles.push(said(partd(&mut guest)?.profile(*id, name.clone()))?);
        }
        kept.push((definition, profiles));
    }
    if kept[1].0 != changed
        || kept[0].0.default_profile != "NORMAL"
        || kept.iter().any(|(_, profiles)| profiles.len() != 2)
    {
        return Err(format!("the definitions as they were kept: {kept:?}"));
    }
    // Each has a serial number of its own, of the host's five
    // characters and its ID.
    let serials: Vec<&str> = kept.iter().map(|(d, _)| d.serial.as_str()).collect();
    let host_serial = channel(&mut guest)?
        .status()?
        .fact(release::fact::SERIAL)
        .unwrap_or_default()
        .to_string();
    for (i, serial) in serials.iter().enumerate() {
        let expected = model::definition::serial_for(&host_serial, ids[i]);
        if *serial != expected {
            return Err(format!(
                "partition {} of the host {host_serial} has the serial number {serial}, not \
                 {expected}",
                ids[i]
            ));
        }
    }

    // A bad profile: every error at once, each a message, and nothing
    // of it is kept.
    let mut bad = spin(0, 4).shared(10, Sharing::Capped).profile();
    bad.name = "bad one".into();
    bad.processors.minimum = 6;
    bad.processors.weight = Some(999);
    bad.memory.minimum_mib = 1000;
    bad.memory.desired_mib = 100;
    bad.scsi[0].slot = 1;
    bad.scsi[0].luns[0].backing = "relative/path".into();
    let errors = partd(&mut guest)?
        .put_profile(ids[0], bad)
        .map_err(|e| e.to_string())?
        .err()
        .ok_or("a bad profile was kept")?;
    let got: Vec<&str> = errors.iter().map(|e| e.id.as_str()).collect();
    for wanted in [
        &qvsmsg::PROFILE_NAME_INVALID,
        &qvsmsg::PROCESSORS_ORDER,
        &qvsmsg::UNITS_TOO_FEW,
        &qvsmsg::WEIGHT_INVALID,
        &qvsmsg::MEMORY_ORDER,
        &qvsmsg::MEMORY_BLOCK,
        &qvsmsg::MEMORY_RANGE,
        &qvsmsg::SLOT_INVALID,
        &qvsmsg::BACKING_INVALID,
    ] {
        if !got.contains(&wanted.id) {
            return Err(format!(
                "a profile wrong in nine ways was refused with {got:?}: no {}",
                wanted.id
            ));
        }
    }
    if errors
        .iter()
        .any(|e| e.text.is_empty() || e.help.is_empty())
    {
        return Err("a message came without its text".into());
    }
    if said(partd(&mut guest)?.profiles(ids[0]))?.len() != 2 {
        return Err("a refused profile was kept".into());
    }
    // What cannot be: a second partition of one name, a definition
    // changed in what is fixed.
    let twice = partd(&mut guest)?
        .create("RENAMED".into())
        .map_err(|e| e.to_string())?
        .err()
        .map(|errors| manager::refused(&errors))
        .unwrap_or_default();
    let mut fixed = kept[2].0.clone();
    fixed.serial = "AAAAAAA".into();
    let fixed = partd(&mut guest)?
        .change(fixed)
        .map_err(|e| e.to_string())?
        .err()
        .map(|errors| manager::refused(&errors))
        .unwrap_or_default();
    if !twice.starts_with(qvsmsg::NAME_IN_USE.id) || !fixed.starts_with(qvsmsg::FIELD_FIXED.id) {
        return Err(format!(
            "a second partition of one name: `{twice}`; a serial number changed: `{fixed}`"
        ));
    }

    // The host restarts: everything is as it was kept.
    channel(&mut guest)?.tell(command::POWER_OFF)?;
    let limit = Instant::now() + 3 * run.timeout;
    guest.expect(&posted(&refcode::POWER_OFF, ""), limit)?;
    guest.wait_exit(limit)?;
    drop(guest);
    let mut guest = host(run, &l)?;
    guest.seen(
        &posted(&refcode::PARTD_STARTED, "3 defined, 0 running"),
        limit,
    )?;
    let listed = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
    if listed.len() != 3 || listed.iter().any(|p| p.state != State::NotActivated) {
        return Err(format!("after the restart the partitions are {listed:?}"));
    }
    for (i, id) in ids.iter().enumerate() {
        let definition = said(partd(&mut guest)?.definition(*id))?;
        let mut profiles = Vec::new();
        for name in said(partd(&mut guest)?.profiles(*id))? {
            profiles.push(said(partd(&mut guest)?.profile(*id, name))?);
        }
        if (definition.clone(), profiles.clone()) != kept[i] {
            return Err(format!(
                "partition {id} after the restart: {definition:?} with {profiles:?}, kept as \
                 {:?}",
                kept[i]
            ));
        }
    }
    // A deleted partition's ID is free again; one that exists keeps its own.
    said(partd(&mut guest)?.delete(ids[1]))?;
    let again = said(partd(&mut guest)?.create("FOURTH".into()))?;
    if again.id != ids[1] || again.serial != kept[1].0.serial {
        return Err(format!(
            "after partition {} was deleted, a new one is {again:?}",
            ids[1]
        ));
    }
    let files = channel(&mut guest)?.ls("/data/state/partitions")?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "three partitions (serial numbers {}) with two profiles each, one renamed with keylock \
         manual, another default profile and auto start settings: all read back the same \
         after a host restart (`C2006000 Partition manager started: 3 defined, 0 running`); a \
         profile wrong in nine ways refused with {} messages ({}), nothing kept; a second \
         partition of one name and a changed serial number refused; state under \
         /data/state/partitions: {}; weights kept {:?}",
        serials.join(" "),
        errors.len(),
        got.join(" "),
        files.join(" "),
        kept.iter()
            .map(|(_, p)| p[1].processors.weight.unwrap_or(0))
            .collect::<Vec<u32>>(),
    ))
}
