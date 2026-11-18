// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A host at its limits. One partition is activated with every device
//! limit of a profile reached at once: eight SCSI adapters, the first
//! with 64 units, eight Ethernet adapters each with 32 tagged VLANs on a
//! switch of 256 VLANs. One past each is refused with its message, as are
//! a seventeenth switch, a switch's 257th VLAN and a 65th partition.

use std::time::{Duration, Instant};

use model::{Lun, Profile, ScsiAdapter};
use partd::{Calls as _, State};
use poold::proto::Calls as _;
use vswitchd::proto::Calls as _;

use crate::qemu::image::blank;
use crate::qemu::session::Session;
use crate::suite::base::Run;
use crate::suite::network::{
    Rig, adapter, boot, guest_spec, ports, set_switches, switch, vswitchd,
};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::storage::{POOL_DISK_BYTES, disk_names, pool_until, poold};

/// The switch the adapters are on: VLANs 1 to 256.
const WIDE: &str = "WIDE";

/// `result` refused with message `id`, and not accepted.
fn refused_with<T>(
    what: &str,
    result: Result<Result<T, Vec<qvsmsg::Issued>>, ipc::WireError>,
    id: &str,
) -> Result<(), String> {
    match result {
        Ok(Ok(_)) => Err(format!("{what}: accepted, one past the limit")),
        Ok(Err(errors)) if errors.iter().any(|e| e.id == id) => Ok(()),
        Ok(Err(errors)) => Err(format!(
            "{what}: refused without {id}: {}",
            manager::refused(&errors)
        )),
        Err(e) => Err(format!("{what}: {e}")),
    }
}

/// A unit of `backing`.
fn unit(lun: u32, backing: String) -> Lun {
    Lun {
        lun,
        backing,
        serial: None,
    }
}

/// The cheap guest's profile with every device limit reached; its
/// further units are the virtual disks `vdisks` (69 of them).
fn at_limits(vdisks: &[String]) -> Profile {
    let tagged: Vec<u32> = (2..=33).collect();
    let adapters = (0..8).map(|i| adapter(10 + i, WIDE, 1, &tagged)).collect();
    let mut spec = guest_spec(0, adapters);
    spec.name = "LIMITS".into();
    let mut p = spec.profile();
    let mut more = vdisks.iter().cloned();
    for lun in 2..64u32 {
        p.scsi[0]
            .luns
            .push(unit(lun, more.next().unwrap_or_default()));
    }
    for slot in 3..10u32 {
        p.scsi.push(ScsiAdapter {
            slot,
            luns: vec![unit(0, more.next().unwrap_or_default())],
        });
    }
    p
}

/// `full` named PAST and changed by `change`.
fn past(full: &Profile, change: impl FnOnce(&mut Profile)) -> Profile {
    let mut p = full.clone();
    p.name = "PAST".into();
    change(&mut p);
    p
}

/// Switches: 256 VLANs on WIDE, not 257; sixteen switches, not
/// seventeen.
fn switches(g: &mut Session) -> Result<(), String> {
    let vlans: Vec<u16> = (1..=256).collect();
    set_switches(g, &[switch(WIDE, 1, &vlans, 1500, None)])?;
    let mut more = vlans.clone();
    more.push(257);
    refused_with(
        "a switch's 257th VLAN",
        vswitchd(g)?.change(WIDE.into(), more, 1500),
        qvsmsg::SWITCH_VLANS_TOO_MANY.id,
    )?;
    for n in 2..=16 {
        vswitchd(g)?
            .create(format!("LIM{n:02}"), vec![1], 1500)
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("switch {n} of 16: {}", manager::refused(&e)))?;
    }
    refused_with(
        "a seventeenth switch",
        vswitchd(g)?.create("LIM17".into(), vec![1], 1500),
        qvsmsg::SWITCHES_TOO_MANY.id,
    )
}

/// The suite.
pub fn limits(run: &Run<'_>) -> Result<String, String> {
    let pool = run.file("limits-pool.img");
    blank(&pool, POOL_DISK_BYTES)?;
    let mut rig = Rig::new(0, 1);
    rig.disks = vec![pool];
    let mut net = boot(run, &rig)?;
    let g = &mut net.guest;
    // The host's seed, the guest's disk and seed, then the pool's disk.
    let names = disk_names(g, 4)?;
    crate::suite::storage::said(poold(g)?.create_pool(
        "LIM".into(),
        "none".into(),
        vec![names[3].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(g, "LIM", run.deadline(), |p| !p.mount.is_empty())?;
    let mut vdisks = Vec::new();
    for n in 0..69 {
        let name = format!("U{n:02}");
        crate::suite::storage::said(poold(g)?.create_vdisk("LIM".into(), name.clone(), 64, false))?;
        vdisks.push(format!("LIM/{name}"));
    }

    switches(g)?;

    // A profile one past each device limit.
    let id = manager::partition(g, "LIMITS")?;
    let full = at_limits(&vdisks);
    let checks = [
        (
            "a ninth SCSI adapter",
            past(&full, |p| {
                p.scsi.push(ScsiAdapter {
                    slot: 30,
                    luns: vec![unit(0, "LIM/EXTRA".into())],
                });
            }),
            qvsmsg::TOO_MANY_ADAPTERS.id,
        ),
        (
            "a 65th unit of an adapter",
            past(&full, |p| p.scsi[0].luns.push(unit(64, "LIM/EXTRA".into()))),
            qvsmsg::TOO_MANY_UNITS.id,
        ),
        (
            "a ninth Ethernet adapter",
            past(&full, |p| p.ethernet.push(adapter(18, WIDE, 1, &[]))),
            qvsmsg::ETHERNET_TOO_MANY.id,
        ),
        (
            "an adapter's 33rd tagged VLAN",
            past(&full, |p| p.ethernet[0].tagged.push(34)),
            qvsmsg::TAGGED_INVALID.id,
        ),
    ];
    for (what, profile, message) in checks {
        refused_with(what, partd(g)?.put_profile(id, profile), message)?;
    }

    // The profile at every limit, activated and running.
    said(partd(g)?.put_profile(id, full))?;
    said(partd(g)?.activate(id, "LIMITS".into(), None, None))?;
    let status = manager::until(g, id, State::Running, run.deadline())?;
    let mine: Vec<_> = ports(g)?
        .into_iter()
        .filter(|p| p.partition == id)
        .collect();
    if mine.len() != 8 || mine.iter().any(|p| p.tagged.len() != 32) {
        return Err(format!("the running partition's ports: {mine:?}"));
    }
    let held = poold(g)?
        .holders()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|h| h.partition == id)
        .count();
    said(partd(g)?.shut_down_as(id, partd::proto::Shutdown::Immediate, 0, false))?;
    manager::ended(g, id, Instant::now() + Duration::from_mins(2))?;

    // Partitions: 64 defined, not 65.
    let defined = partd(g)?.list().map_err(|e| e.to_string())?.len();
    for n in defined + 1..=64 {
        manager::partition(g, &format!("P{n:02}"))?;
    }
    refused_with(
        "a 65th partition",
        partd(g)?.create("P65".into()),
        qvsmsg::TOO_MANY_PARTITIONS.id,
    )?;
    Ok(format!(
        "LIMITS ran (`{}`) with 8 SCSI adapters, 64 units on the first and {held} held of \
         the pool, and 8 Ethernet adapters of 32 tagged VLANs each on a switch of 256; then \
         16 switches and 64 partitions; one past each of the seven limits refused with its \
         message",
        status.code
    ))
}
