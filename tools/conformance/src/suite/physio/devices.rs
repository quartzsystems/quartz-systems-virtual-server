// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Physical I/O on a host with QEMU's IOMMU and interrupt remapping and
//! devices to give away: a network adapter and an `NVMe` controller each
//! in a group of its own, and a pair of functions that share one. The
//! partitions' system is a copy of the test image, whose guest probe
//! says on its console what it was given (`lic/qsvsinit`'s hooks), and
//! the host relays its lines.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use model::profile::{IplSource, PhysicalIo, Profile};
use partd::{Calls as _, State};

use crate::qemu::image::blank;
use crate::qemu::session::Session;
use crate::qemu::{PASSED_NIC, PASSED_NVME, PASSED_PAIR, Passed};
use crate::suite::base::{Guest, Run, posted};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::partition::{QSVS_MEMORY_MIB, fresh_host, host, launch, unit};

/// The network adapter's MAC address.
pub const NIC_MAC: &str = "52:54:00:9a:00:01";
/// The `NVMe` controller's serial number.
pub const NVME_SERIAL: &str = "QSVSNVME1";
/// Bytes of the `NVMe` controller's namespace.
const NVME_BYTES: u64 = 64 << 20;

/// The devices every suite here gives the host.
pub fn passed(run: &Run<'_>) -> Result<Vec<Passed>, String> {
    // Blank at each rig: what a partition finds on it is this run's.
    let nvme = run.file("nvme.img");
    blank(&nvme, NVME_BYTES)?;
    let pair = [run.file("pair0.img"), run.file("pair1.img")];
    for p in &pair {
        blank(p, NVME_BYTES)?;
    }
    Ok(vec![
        Passed::Nic {
            place: PASSED_NIC,
            mac: NIC_MAC.into(),
        },
        Passed::Nvme {
            place: PASSED_NVME,
            path: nvme,
            serial: NVME_SERIAL.into(),
        },
        Passed::Pair {
            place: PASSED_PAIR,
            paths: pair,
        },
    ])
}

/// A fresh host with the IOMMU and the devices, the test image's copy
/// as the partitions' unit, `more` after it.
pub fn rig<'a>(run: &Run<'a>, more: &[PathBuf]) -> Result<Guest<'a>, String> {
    fresh_host(run)?;
    let mut units = vec![unit(run, run.disk)?];
    units.extend_from_slice(more);
    let mut l = launch(run, &units)?;
    l.iommu = true;
    l.passed = passed(run)?;
    host(run, &l)
}

/// The location code of the host's function at `place`, function
/// `function`, of class `class` (`0108` for `NVMe`).
pub fn located(guest: &mut Session, place: u8, function: u8) -> Result<String, String> {
    let address = format!("0000:00:{place:02x}.{function}");
    let devices = partd(guest)?
        .devices()
        .map_err(|e| format!("the partition manager: {e}"))?;
    devices
        .iter()
        .find(|d| d.address == address)
        .map(|d| d.location.clone())
        .ok_or_else(|| format!("no device at {address}: {devices:?}"))
}

/// Release the device at `location`, typed in full: the locations
/// released.
pub fn release(guest: &mut Session, location: &str) -> Result<Vec<String>, String> {
    said(partd(guest)?.release(location.into(), location.into()))
}

/// A profile of the test image's copy with `devices`, each required or
/// not.
pub fn profile(name: &str, devices: &[(&str, bool)]) -> Profile {
    profile_on(name, devices, crate::suite::partition::UNIT)
}

/// The same, the copy on the host's disk `unit`.
pub fn profile_on(name: &str, devices: &[(&str, bool)], unit: &str) -> Profile {
    let mut spec = Spec::new(1, QSVS_MEMORY_MIB, unit);
    spec.side = IplSource::A;
    spec.name = name.into();
    let mut p = spec.profile();
    p.physical_io = devices
        .iter()
        .map(|(location, required)| PhysicalIo {
            location: (*location).into(),
            required: *required,
        })
        .collect();
    p
}

/// Put `p` for `id` and activate it.
pub fn activate(guest: &mut Session, id: u32, p: Profile) -> Result<(), String> {
    let name = p.name.clone();
    said(partd(guest)?.put_profile(id, p))?;
    said(partd(guest)?.activate(id, name, None, None)).map(|_| ())
}

/// Shut `id` down at once and wait until it has ended.
pub fn end(guest: &mut Session, id: u32) -> Result<(), String> {
    said(partd(guest)?.shut_down(id))?;
    manager::ended(guest, id, Instant::now() + Duration::from_mins(2)).map(|_| ())
}

/// The line the partition `name`'s probe said with `needle`.
pub fn probed(
    guest: &mut Session,
    name: &str,
    needle: &str,
    run: &Run<'_>,
) -> Result<String, String> {
    // The probe's lines are whole once it has said it is done.
    let deadline = run.deadline() + Duration::from_mins(3);
    guest.seen(&format!("{name}| PIO DONE"), deadline)?;
    let line = guest.line_with(&format!("{name}| PIO {needle}"), deadline)?;
    Ok(line
        .split_once("| ")
        .map_or(line.as_str(), |(_, l)| l)
        .to_string())
}

/// Suite 4: the `NVMe` controller given to a partition: its system sees
/// the disk and writes to it; after the partition ends the device is
/// reset, and a second partition gets it clean.
pub fn nvme(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, &[])?;
    let g = &mut guest;
    let nvme = located(g, PASSED_NVME, 0)?;
    let released = release(g, &nvme)?;
    if released != [nvme.clone()] {
        return Err(format!("released {released:?}"));
    }
    let first = manager::partition(g, "FIRST")?;
    let second = manager::partition(g, "SECOND")?;
    activate(g, first, profile("IO", &[(&nvme, true)]))?;
    g.expect(
        &posted(
            &refcode::DEVICE_ASSIGNED,
            &format!("{nvme}, partition FIRST"),
        ),
        run.deadline(),
    )?;
    let one = probed(g, "FIRST", "DISK nvme", run)?;
    if !one.contains("found none") || !one.ends_with("read back ok") {
        return Err(format!("FIRST: {one}"));
    }
    end(g, first)?;
    g.expect(
        &posted(
            &refcode::DEVICE_RETURNED,
            &format!("{nvme}, partition FIRST"),
        ),
        run.deadline(),
    )?;
    let serial = said(partd(g)?.definition(first))?.serial;
    activate(g, second, profile("IO", &[(&nvme, true)]))?;
    let two = probed(g, "SECOND", "DISK nvme", run)?;
    if !two.contains(&format!("found QSVS-PIO {serial}")) || !two.ends_with("read back ok") {
        return Err(format!("SECOND: {two}"));
    }
    let listed = probed(g, "SECOND", "GUEST", run)?;
    end(g, second)?;
    manager::until(g, second, State::NotActivated, run.deadline())?;
    Ok(format!(
        "{nvme} released alone; FIRST's system: `{one}`; ended, the controller reset and \
         given back (C2013035); SECOND's: `{listed}`, `{two}`"
    ))
}
