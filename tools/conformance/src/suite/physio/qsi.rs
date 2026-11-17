// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 15: physical I/O, tape pass-through and entitlement over QSI
//! and from `qsictl`, on an installed host with its server: the devices
//! and drives as a console sees them, a partition made, given the
//! controller and the drive and activated by the console, a second
//! refused the held controller, its entitlement changed and refused,
//! the events of each, and `qsictl io`, `tape` and `partition
//! entitlement` from the build machine.

use std::time::{Duration, Instant};

use model::profile::MediaDevice;
use model::{Sharing, units::Units};
use partd::Calls as _;
use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::host::{host as h, partition, profiles, vres};
use qsictl::link::Link;
use qsvshook::proto::command;

use super::devices::{located, passed, probed, profile};
use crate::qemu::PASSED_NVME;
use crate::suite::base::{Run, channel};
use crate::suite::install::{Kind, boot, installed, machine};
use crate::suite::partition::manager::{partd, said};
use crate::suite::qsi::families::{ask, refused};
use crate::suite::qsi::flows::{gather, qsictl_binary, subscribe};
use crate::suite::qsi::{Qsi, free_port, link, pair, start_pairing, to_pairing};

/// The host's management NIC.
const MANAGEMENT_MAC: &str = "52:54:00:12:34:56";
/// The tape device's slot.
const SLOT: u32 = 3;

/// An installed host with its server, the IOMMU, the devices, the test
/// image's copy as `/dev/vdb` and the tape double made.
fn host<'a>(run: &Run<'a>) -> Result<Qsi<'a>, String> {
    let disks = installed(run, Kind::Single)?;
    let unit = crate::suite::partition::unit(run, run.disk)?;
    let mut l = machine(run, &[Some(disks[0].as_path())]);
    let port = free_port()?;
    l.nic = Some(MANAGEMENT_MAC.into());
    l.keys = true;
    l.qsi = Some(port);
    l.iommu = true;
    l.passed = passed(run)?;
    l.memory_mb = crate::suite::partition::HOST_MEMORY_MB;
    l.cpus = crate::suite::partition::HOST_CPUS;
    l.prealloc = true;
    l.extra.push(crate::qemu::Drive {
        path: unit,
        serial: "QSVSUNIT0".into(),
        place: Some(4),
    });
    let mut guest = boot(run, &l)?;
    guest.seen(&refcode::QSI_STARTED.tag(), run.deadline())?;
    for (file, value) in [("ptype", "1"), ("add_host", "1")] {
        channel(&mut guest)?.ask(&format!(
            "{} /sys/bus/pseudo/drivers/scsi_debug/{file} {value}",
            command::WRITE
        ))?;
    }
    Ok(Qsi {
        guest,
        address: format!("127.0.0.1:{port}"),
        launch: l,
    })
}

/// The devices as QSI has them.
fn devices(l: &mut Link) -> Result<Vec<h::Device>, String> {
    match ask(l, AnyRequest::Host(h::Request::Devices(Empty)))? {
        AnyReply::Host(h::Reply::Devices(d)) => Ok(d),
        other => Err(format!("devices: {other:?}")),
    }
}

/// The physical tape drives as QSI has them.
fn drives(l: &mut Link) -> Result<Vec<vres::TapeDrive>, String> {
    match ask(l, AnyRequest::Vres(vres::Request::TapeDrives(Empty)))? {
        AnyReply::Vres(vres::Reply::TapeDrives(d)) => Ok(d),
        other => Err(format!("tape drives: {other:?}")),
    }
}

/// Make partition `name` over QSI with `p` as its profile: its ID.
fn made(l: &mut Link, name: &str, p: &model::profile::Profile) -> Result<u32, String> {
    let AnyReply::Partition(partition::Reply::Create(d)) = ask(
        l,
        AnyRequest::Partition(partition::Request::Create(partition::Create {
            name: name.into(),
        })),
    )?
    else {
        return Err("create".into());
    };
    ask(
        l,
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id: d.id,
            profile: qsiserver::families::convert::profile(p),
            revision: None,
        })),
    )?;
    Ok(d.id)
}

fn activate(id: u32, name: &str) -> AnyRequest {
    AnyRequest::Partition(partition::Request::Activate(partition::Activate {
        id,
        profile: name.into(),
        source: None,
        keylock: None,
    }))
}

fn entitlement(id: u32, units: u32) -> AnyRequest {
    AnyRequest::Partition(partition::Request::Entitlement(partition::Entitlement {
        id,
        units: Some(units),
        sharing: None,
        weight: None,
    }))
}

/// Suite 15.
#[allow(clippy::too_many_lines, clippy::many_single_char_names)] // a request a step, in order
pub fn over_qsi(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run)?;
    let deadline = run.deadline();
    to_pairing(&mut q, deadline)?;
    let (password, _) = start_pairing(&mut q, deadline)?;
    // `qsictl` pairs first, as the build machine; the harness's console
    // second.
    let dir = run.file("qsictl");
    let bin = qsictl_binary()?;
    let qsictl = |args: &[&str]| -> Result<String, String> {
        let out = std::process::Command::new(&bin)
            .arg("--dir")
            .arg(&dir)
            .args(args)
            .output()
            .map_err(|e| format!("qsictl: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!(
                "qsictl {args:?}: {}{text}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    };
    qsictl(&["init"])?;
    qsictl(&["pair", &q.address, &password, "--as", "vs"])?;
    // A second pairing from option 4's menu.
    q.guest.type_keys(crate::qemu::term::key::F12)?;
    crate::suite::install::titled(&mut q.guest, crate::suite::qsi::PAIRING, deadline)?;
    let (password, _) = start_pairing(&mut q, deadline)?;
    let id = crate::suite::qsi::identity("PIO")?;
    let fingerprint = pair(&q, &id, &password, "PIO")?.fingerprint;
    let mut l = link(&q, &id, fingerprint)?;
    let since = subscribe(&mut l, None)?;
    let g = &mut q.guest;
    // The devices, as the partition manager has them.
    let nvme = located(g, PASSED_NVME, 0)?;
    let local = partd(g)?.devices().map_err(|e| e.to_string())?;
    let listed = devices(&mut l)?;
    let mut a: Vec<&str> = local.iter().map(|d| d.location.as_str()).collect();
    let mut b: Vec<&str> = listed.iter().map(|d| d.location.as_str()).collect();
    a.sort_unstable();
    b.sort_unstable();
    if a != b {
        return Err(format!("QSI's devices {b:?}, the host's {a:?}"));
    }
    let before = listed
        .iter()
        .find(|d| d.location == nvme)
        .ok_or("the controller is not listed")?
        .clone();
    if before.released || before.assignable {
        return Err(format!("before its release: {before:?}"));
    }
    // Released at the host, as only the host may.
    said(partd(g)?.release(nvme.clone(), nvme.clone()))?;
    let drive = partd(g)?
        .tape_drives()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.product == "scsi_debug")
        .ok_or("the double is not listed")?;
    said(partd(g)?.release(drive.location.clone(), drive.location.clone()))?;
    let after = devices(&mut l)?
        .into_iter()
        .find(|d| d.location == nvme)
        .ok_or("the controller is gone from QSI's list")?;
    if !after.released || !after.assignable {
        return Err(format!("after its release: {after:?}"));
    }
    let tape = drives(&mut l)?;
    if !tape
        .iter()
        .any(|d| d.location == drive.location && d.released)
    {
        return Err(format!("QSI's tape drives: {tape:?}"));
    }
    // A partition with both, shared and capped at 0.50 of 0.20 to 1.00.
    let mut p = profile("IO", &[(&nvme, true)]);
    p.processors.mode = model::ProcessorMode::Shared;
    p.processors.minimum_units = Some(Units(20));
    p.processors.desired_units = Some(Units(50));
    p.processors.maximum_units = Some(Units(100));
    p.processors.sharing = Some(Sharing::Capped);
    p.tape = vec![MediaDevice {
        slot: SLOT,
        media: Some(format!("{}{}", model::validate::DRIVE_MARK, drive.location)),
    }];
    let io = made(&mut l, "IO", &p)?;
    let other = made(&mut l, "OTHER", &profile("IO", &[(&nvme, true)]))?;
    ask(&mut l, activate(io, "IO"))?;
    let disk = probed(g, "IO", "DISK nvme", run)?;
    let st = probed(g, "IO", "TAPE st0 write", run)?;
    if !disk.ends_with("read back ok") || !st.contains("read back ok") {
        return Err(format!("the system's devices: `{disk}`, `{st}`"));
    }
    let held = refused(&mut l, &activate(other, "IO"))?;
    let changed = match ask(&mut l, entitlement(io, 100))? {
        AnyReply::Partition(partition::Reply::Entitlement(c)) => c,
        other => return Err(format!("entitlement: {other:?}")),
    };
    if changed.units != 100 {
        return Err(format!("IO's units after the change: {changed:?}"));
    }
    let outside = refused(&mut l, &entitlement(io, 120))?;
    let holders = drives(&mut l)?;
    if !holders
        .iter()
        .any(|d| d.location == drive.location && d.holder.partition == io)
    {
        return Err(format!("the drive's holder over QSI: {holders:?}"));
    }
    let events = gather(&mut l, Duration::from_secs(5))?;
    let seen = |word: &str| {
        events
            .iter()
            .filter(|(n, e)| *n >= since.next && format!("{e:?}").contains(word))
            .count()
    };
    let (device_events, drive_events, entitlement_events) = (
        seen("Device"),
        seen("TapeDrive"),
        seen("EntitlementChanged"),
    );
    if device_events == 0 || drive_events == 0 || entitlement_events == 0 {
        return Err(format!(
            "events: {device_events} of devices, {drive_events} of drives, \
             {entitlement_events} of entitlement in {events:?}"
        ));
    }
    l.close();
    // The build machine's console.
    let io_out = qsictl(&["io"])?;
    let tape_out = qsictl(&["tape"])?;
    let lowered = qsictl(&[
        "partition",
        "entitlement",
        &io.to_string(),
        "--units",
        "0.50",
    ])?;
    if !io_out.contains(&nvme) || !tape_out.contains(&drive.location) || !lowered.contains("50") {
        return Err(format!(
            "qsictl io:\n{io_out}\nqsictl tape:\n{tape_out}\nqsictl partition entitlement:\n{lowered}"
        ));
    }
    let deadline = Instant::now() + Duration::from_mins(2);
    said(partd(g)?.shut_down(io))?;
    crate::suite::partition::manager::ended(g, io, deadline)?;
    Ok(format!(
        "QSI's devices the host's {} by location; {nvme} not released, then released and \
         assignable; the double {} released; IO made, given both and activated over QSI: \
         `{disk}`, `{st}`; OTHER refused {held}; IO's units to {:?} over QSI, 1.20 refused \
         {outside}; the drive held by IO over QSI; {device_events} device, {drive_events} \
         drive and {entitlement_events} entitlement event(s); qsictl io, tape and partition \
         entitlement --units 0.50 answered (qsictl io: {} lines)",
        a.len(),
        drive.location,
        changed.units,
        io_out.lines().count()
    ))
}
