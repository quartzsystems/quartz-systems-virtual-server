// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Physical I/O's rules as the host keeps them: what can never be
//! released and why, a group that goes whole, a host without the IOMMU,
//! a device held, a VMM killed and a reset that fails, a restart and a
//! device taken out of the machine.

use std::time::Duration;

use partd::{Calls as _, State};
use poold::proto::Calls as _;
use qsvshook::proto::command;

use super::devices::{activate, end, located, passed, probed, profile, profile_on, release};
use crate::qemu::image::blank;
use crate::qemu::session::Session;
use crate::qemu::{Far, PASSED_NIC, PASSED_NVME, PASSED_PAIR, PASSED_SHARED, Passed, UplinkNic};
use crate::suite::base::{Guest, Run, channel, posted};
use crate::suite::network::{set_switches, switch, uplink_location, uplink_mac};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::partition::{fresh_host, host, launch, signal_to, unit};
use crate::suite::storage::{pool_until, poold};

/// The management interface's MAC address.
const MANAGEMENT_MAC: &str = "52:54:00:12:34:56";
/// The serial number of the disk that shares a group with an adapter.
const SHARED_SERIAL: &str = "QSVSSHARED";

/// The devices: the suites' own and the disk that shares a group.
fn all_passed(run: &Run<'_>) -> Result<Vec<Passed>, String> {
    let mut all = passed(run)?;
    let shared = run.file("shared.img");
    blank(&shared, 1 << 30)?;
    let beside = run.file("beside.img");
    blank(&beside, 64 << 20)?;
    all.push(Passed::Shared {
        place: PASSED_SHARED,
        path: shared,
        serial: SHARED_SERIAL.into(),
        beside,
    });
    Ok(all)
}

/// The second copy of the test image: `/dev/vdc`, for a partition
/// beside the first.
const SECOND_UNIT: &str = "/dev/vdc";

/// A host with the IOMMU or without it, its management NIC, an uplink
/// NIC, the devices, two copies of the test image as units.
fn rig<'a>(run: &Run<'a>, iommu: bool, devices: Vec<Passed>) -> Result<Guest<'a>, String> {
    fresh_host(run)?;
    let second = run.file("unit2.img");
    crate::qemu::image::sparse_copy(run.disk, &second)?;
    let units = vec![unit(run, run.disk)?, second];
    let mut l = launch(run, &units)?;
    l.iommu = iommu;
    l.passed = devices;
    l.nic = Some(MANAGEMENT_MAC.into());
    l.uplinks = vec![UplinkNic {
        mac: uplink_mac(0),
        far: Far::User(None),
        mtu: None,
    }];
    host(run, &l)
}

/// Wait until the problem service's book has a problem of `code`.
pub(crate) fn problem_of(guest: &mut Session, code: &refcode::RefCode) -> Result<(), String> {
    let until = std::time::Instant::now() + Duration::from_mins(2);
    loop {
        let book = channel(guest)?.text(PROBLEMS).unwrap_or_default();
        if book.contains(code.code) {
            return Ok(());
        }
        if std::time::Instant::now() >= until {
            return Err(format!("no problem of {}: {book}", code.code));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// The problem service's book.
const PROBLEMS: &str = "/data/problems/problems.json";

/// The device at `location`, as the partition manager has it.
fn device(guest: &mut Session, location: &str) -> Result<pio::Device, String> {
    partd(guest)?
        .devices()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.location == location)
        .ok_or_else(|| format!("no device {location}"))
}

/// The first reason `location` cannot be released, and that a release
/// is refused with it.
fn not_releasable(guest: &mut Session, location: &str) -> Result<qvsmsg::Issued, String> {
    let d = device(guest, location)?;
    let why = d
        .releasable
        .first()
        .cloned()
        .ok_or_else(|| format!("{location} can be released: {d:?}"))?;
    match partd(guest)?.release(location.into(), location.into()) {
        Ok(Err(refused)) if refused.first() == Some(&why) => Ok(why),
        other => Err(format!("{location}: a release answered {other:?}")),
    }
}

/// Suite 5: what cannot be released: the load source's controller, a
/// pool member's controller, the management interface, an uplink, each
/// with its reason; and a device whose group holds one of them.
pub fn refused(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, true, all_passed(run)?)?;
    let g = &mut guest;
    // The pool on the disk that shares a group with an adapter.
    let disks = poold(g)?.disks().map_err(|e| e.to_string())?;
    let shared_disk = disks
        .iter()
        .find(|d| d.serial == SHARED_SERIAL)
        .ok_or("the shared disk is not there")?;
    crate::suite::storage::said(poold(g)?.create_pool(
        "POOL1".into(),
        "none".into(),
        vec![shared_disk.name.clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(g, "POOL1", run.deadline(), |p| !p.mount.is_empty())?;
    // The uplink.
    let up = uplink_location(g, 0)?;
    let uplink = vswitch::Uplink {
        interfaces: vec![up.clone()],
        bond: None,
        native_vlan: None,
    };
    set_switches(g, &[switch("PRODNET", 1, &[1], 1500, Some(uplink))])?;
    let disk_controller = |d: &poold::proto::Disk| d.location.clone();
    let load_source = disks
        .iter()
        .find(|d| d.what.starts_with("Load source"))
        .map(disk_controller)
        .ok_or("no load-source disk")?;
    let pool_member = format!("P0000-B00-D{PASSED_SHARED:02X}-F0");
    let management = crate::suite::network::location_of(g, MANAGEMENT_MAC)?;
    let strip = |l: &str| {
        l.rsplit_once("-E")
            .map_or_else(|| l.to_string(), |(c, _)| c.to_string())
    };
    let mut said_why = Vec::new();
    for (what, location, word) in [
        ("load source", load_source.clone(), "load-source"),
        ("pool member", pool_member.clone(), "POOL1"),
        ("management", strip(&management), "management"),
        ("uplink", strip(&up), "PRODNET"),
    ] {
        let why = not_releasable(g, &location)?;
        if !why.is(&qvsmsg::DEVICE_HOST_USES) || !why.text.contains(word) {
            return Err(format!("{what} {location}: {why}"));
        }
        said_why.push(format!("{what} {location}: `{}`", why.text));
    }
    // The controller in the pool member's group.
    let beside = format!("P0000-B00-D{PASSED_SHARED:02X}-F1");
    let why = not_releasable(g, &beside)?;
    if !why.is(&qvsmsg::DEVICE_GROUP_HOST) || !why.text.contains(&pool_member) {
        return Err(format!("{beside}: {why}"));
    }
    said_why.push(format!("{beside}: `{}`", why.text));
    Ok(said_why.join("; "))
}

/// Suite 6: the shared group: releasing one function releases both; a
/// profile that names only one is refused by name.
pub fn group(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, true, passed(run)?)?;
    let g = &mut guest;
    let f0 = located(g, PASSED_PAIR, 0)?;
    let f1 = located(g, PASSED_PAIR, 1)?;
    let with = device(g, &f1)?.with;
    if with != [f0.clone()] {
        return Err(format!("{f1} goes with {with:?}"));
    }
    let released = release(g, &f1)?;
    let mut sorted = released.clone();
    sorted.sort();
    if sorted != [f0.clone(), f1.clone()] {
        return Err(format!("released {released:?}"));
    }
    let id = manager::partition(g, "HALF")?;
    let refused = partd(g)?
        .put_profile(id, profile("HALF", &[(&f1, true)]))
        .map_err(|e| e.to_string())?
        .unwrap_err();
    if !refused[0].is(&qvsmsg::DEVICE_GROUP_SPLIT) || !refused[0].text.contains(&f0) {
        return Err(format!("a profile of {f1} alone: {refused:?}"));
    }
    activate(g, id, profile("WHOLE", &[(&f0, true), (&f1, true)]))?;
    let listed = probed(g, "HALF", "GUEST", run)?;
    end(g, id)?;
    Ok(format!(
        "{f1} goes with {f0}; releasing it released {}; a profile of {f1} alone: `{}`; both \
         given whole: `{listed}`",
        released.join(" and "),
        refused[0]
    ))
}

/// Suite 7: a host started without the IOMMU: its code and problem,
/// nothing released, and everything else works.
pub fn iommu_off(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, false, passed(run)?)?;
    let g = &mut guest;
    let off = posted(&refcode::PHYSIO_OFF, "");
    let line = g.line_with(&off[..off.len() - 1], run.deadline())?;
    problem_of(g, &refcode::PHYSIO_OFF)?;
    let nvme = located(g, PASSED_NVME, 0)?;
    let d = device(g, &nvme)?;
    let why = d
        .releasable
        .first()
        .cloned()
        .ok_or("the controller can be released")?;
    if !(why.is(&qvsmsg::NO_IOMMU) || why.is(&qvsmsg::NO_REMAPPING)) {
        return Err(format!("{nvme}: {why}"));
    }
    let refused = said(partd(g)?.release(nvme.clone(), nvme.clone()));
    if refused.is_ok() {
        return Err("a device was released without the IOMMU".into());
    }
    // A partition with no physical I/O runs as before.
    let id = manager::partition(g, "PLAIN")?;
    activate(g, id, profile("PLAIN", &[]))?;
    manager::until(
        g,
        id,
        State::Running,
        run.deadline() + Duration::from_mins(3),
    )?;
    end(g, id)?;
    Ok(format!(
        "`{}`; its problem open; {nvme}: `{}`, a release refused; a partition with no \
         physical I/O ran and ended",
        line.trim(),
        why
    ))
}

/// Suite 8: a device held by a running partition fails a second's
/// activation by name when required, and is left out when not.
pub fn held(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, true, passed(run)?)?;
    let g = &mut guest;
    let nvme = located(g, PASSED_NVME, 0)?;
    let nic = located(g, PASSED_NIC, 0)?;
    release(g, &nvme)?;
    release(g, &nic)?;
    let first = manager::partition(g, "FIRST")?;
    let second = manager::partition(g, "SECOND")?;
    activate(g, first, profile("IO", &[(&nvme, true)]))?;
    probed(g, "FIRST", "DISK nvme", run)?;
    let p = profile_on("IO", &[(&nvme, true), (&nic, true)], SECOND_UNIT);
    said(partd(g)?.put_profile(second, p))?;
    let refused = partd(g)?
        .activate(second, "IO".into(), None, None)
        .map_err(|e| e.to_string())?
        .unwrap_err();
    if !refused[0].is(&qvsmsg::DEVICE_HELD) || !refused[0].text.contains("FIRST") {
        return Err(format!("required and held: {refused:?}"));
    }
    activate(
        g,
        second,
        profile_on("SOME", &[(&nvme, false), (&nic, true)], SECOND_UNIT),
    )?;
    let listed = probed(g, "SECOND", "GUEST", run)?;
    if listed.contains("nvme") {
        return Err(format!("SECOND was given the held controller: {listed}"));
    }
    let holder = device(g, &nvme)?.holder.map(|h| h.name).unwrap_or_default();
    end(g, second)?;
    end(g, first)?;
    Ok(format!(
        "held by FIRST, required by SECOND: `{}`; not required: SECOND activated without it \
         (`{listed}`), FIRST still held it ({holder})",
        refused[0]
    ))
}

/// The file whose addresses the test image's partition manager makes
/// fail their reset.
const NO_RESET: &str = "/run/qsvshook.noreset";

/// Suite 9: a killed `qsvmm` holding a device: the device is reset and
/// given again; a device made to fail its reset is fenced with its
/// problem until the host restarts.
pub fn reset(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, true, passed(run)?)?;
    let nvme = located(&mut guest, PASSED_NVME, 0)?;
    release(&mut guest, &nvme)?;
    let id = manager::partition(&mut guest, "IO")?;
    let p = profile("IO", &[(&nvme, true)]);
    activate(&mut guest, id, p.clone())?;
    probed(&mut guest, "IO", "DISK nvme", run)?;
    signal_to(&mut guest, id, "KILL")?;
    manager::ended(&mut guest, id, run.deadline())?;
    guest.expect(
        &posted(&refcode::DEVICE_RETURNED, &format!("{nvme}, partition IO")),
        run.deadline(),
    )?;
    // Given again after the kill.
    said(partd(&mut guest)?.activate(id, "IO".into(), None, None))?;
    guest.expect(
        &posted(&refcode::DEVICE_ASSIGNED, &format!("{nvme}, partition IO")),
        run.deadline(),
    )?;
    end(&mut guest, id)?;
    // A reset that fails: fenced, with its problem.
    let address = format!("0000:00:{PASSED_NVME:02x}.0");
    channel(&mut guest)?.ask(&format!(
        "{} {NO_RESET} {}",
        command::PUT,
        hex::encode(format!("{address}\n"))
    ))?;
    let refused = partd(&mut guest)?
        .activate(id, "IO".into(), None, None)
        .map_err(|e| e.to_string())?
        .unwrap_err();
    if !refused[0].is(&qvsmsg::DEVICE_NOT_ASSIGNABLE) {
        return Err(format!("a device that did not reset: {refused:?}"));
    }
    let fenced = guest.line_with(refcode::DEVICE_FENCED.code, run.deadline())?;
    problem_of(&mut guest, &refcode::DEVICE_FENCED)?;
    // The fence lasts until the host restarts, whatever the file says.
    channel(&mut guest)?.ask(&format!("{} {NO_RESET}", command::RM))?;
    let still = partd(&mut guest)?
        .activate(id, "IO".into(), None, None)
        .map_err(|e| e.to_string())?;
    if still.is_ok() {
        return Err("a fenced device was given before the host restarted".into());
    }
    channel(&mut guest)?.tell(command::RESTART)?;
    guest.wait_exit(run.deadline())?;
    drop(guest);
    let mut l = launch(run, &[run.file("unit.img")])?;
    l.iommu = true;
    l.passed = passed(run)?;
    l.nic = Some(MANAGEMENT_MAC.into());
    let mut guest = host(run, &l)?;
    guest.seen(&posted(&refcode::DEVICE_BOUND, &nvme), run.deadline())?;
    said(partd(&mut guest)?.activate(id, "IO".into(), None, None))?;
    let again = probed(&mut guest, "IO", "DISK nvme", run)?;
    end(&mut guest, id)?;
    Ok(format!(
        "killed: reset and given back (C2013035), given again (C2013030); a reset made to \
         fail: `{}`, `{}` and its problem; still fenced without the hook; after a restart \
         bound at the IPL and given: `{again}`",
        refused[0],
        fenced.trim()
    ))
}

/// Suite 10: a host restart binds released devices before any host
/// driver; a released device taken out of the machine opens its problem
/// and blocks nothing else.
pub fn restart(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, true, passed(run)?)?;
    let nvme = located(&mut guest, PASSED_NVME, 0)?;
    let nic = located(&mut guest, PASSED_NIC, 0)?;
    release(&mut guest, &nvme)?;
    release(&mut guest, &nic)?;
    channel(&mut guest)?.tell(command::RESTART)?;
    guest.wait_exit(run.deadline())?;
    drop(guest);
    // Again, with the network adapter taken out.
    let mut l = launch(run, &[run.file("unit.img")])?;
    l.iommu = true;
    l.nic = Some(MANAGEMENT_MAC.into());
    l.passed = passed(run)?
        .into_iter()
        .filter(|p| !matches!(p, Passed::Nic { .. }))
        .collect();
    let mut guest = host(run, &l)?;
    let deadline = run.deadline();
    guest.seen(&posted(&refcode::DEVICE_BOUND, &nvme), deadline)?;
    guest.seen(&posted(&refcode::DEVICE_MISSING, &nic), deadline)?;
    let bound = guest.log_position(&posted(&refcode::DEVICE_BOUND, &nvme));
    let started = guest.log_position(refcode::SERVICES_STARTED.code);
    if bound.zip(started).is_none_or(|(b, s)| b > s) {
        return Err(format!(
            "bound at {bound:?}, services started at {started:?}"
        ));
    }
    let driver = channel(&mut guest)?.text(&format!(
        "/sys/bus/pci/devices/0000:00:{PASSED_NVME:02x}.0/uevent"
    ))?;
    if !driver.contains("DRIVER=vfio-pci") {
        return Err(format!("the controller after the IPL: {driver}"));
    }
    problem_of(&mut guest, &refcode::DEVICE_MISSING)?;
    let id = manager::partition(&mut guest, "IO")?;
    activate(
        &mut guest,
        id,
        profile("IO", &[(&nvme, true), (&nic, false)]),
    )?;
    let given = probed(&mut guest, "IO", "DISK nvme", run)?;
    end(&mut guest, id)?;
    Ok(format!(
        "{nvme} bound to vfio-pci at the IPL before the services started; {nic}, taken out, \
         posted its code and opened its problem; a partition with the controller, the \
         adapter not required, ran: `{given}`"
    ))
}
