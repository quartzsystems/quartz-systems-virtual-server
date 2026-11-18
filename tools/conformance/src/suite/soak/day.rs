// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Section 8's soak (`docs/PHASE11.md`): an installed host with QSI, a
//! QS/400 partition and two test image partitions under the soak's load
//! on one switch, for `QSVS_SOAK_MINUTES` (a day by default), while a
//! script works the host: partitions activated, shut down and started
//! again; a console viewer attached; an image uploaded over QSI and
//! deleted; a partition dumped and the dump deleted; a shared
//! partition's entitlement changed; a console paired and removed.
//!
//! What the host has (each service's process, open descriptors and
//! resident memory, its failures, the control groups, links, bridge
//! ports, huge pages, Unix sockets, staging files, problems) is taken
//! after the first activations and again once every partition has
//! ended: no service restarted, no problem the script did not cause,
//! nothing left behind.

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use model::Sharing;
use model::units::Units;
use partd::{Calls as _, State};
use poold::proto::Calls as _;
use resource::Entitlement;

use super::knob;
use crate::qemu::image::{blank, sparse_copy};
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel};
use crate::suite::network::{adapter, set_switches, switch};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::qsi::{Qsi, host_disks, paired};
use crate::suite::storage::{disk_names, pool_until, poold};

/// What the host had: compared before and after.
#[derive(Debug, Default)]
struct Snapshot {
    /// Each service: its process, open descriptors, resident KiB,
    /// failures.
    services: Vec<(String, u32, usize, u64, u32)>,
    cgroups: Vec<String>,
    links: Vec<String>,
    free_pages: u64,
    total_pages: u64,
    unix_sockets: usize,
    staging: Vec<String>,
    problems: Vec<String>,
}

fn number(text: &str) -> u64 {
    text.trim().parse().unwrap_or(0)
}

fn snapshot(guest: &mut Session) -> Result<Snapshot, String> {
    let c = channel(guest)?;
    let status = c.status()?;
    let mut s = Snapshot::default();
    for svc in &status.services {
        let fds = c
            .ls(&format!("/proc/{}/fd", svc.pid))
            .map_or(0, |l| l.len());
        let rss = c
            .text(&format!("/proc/{}/status", svc.pid))
            .unwrap_or_default()
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .map_or(0, |v| number(v.trim().trim_end_matches("kB")));
        s.services
            .push((svc.name.clone(), svc.pid, fds, rss, svc.failures));
    }
    s.cgroups = c.ls("/sys/fs/cgroup/qsvs/partitions").unwrap_or_default();
    s.links = c.ls("/sys/class/net").unwrap_or_default();
    let pages = "/sys/kernel/mm/hugepages/hugepages-2048kB";
    s.free_pages = number(
        &c.text(&format!("{pages}/free_hugepages"))
            .unwrap_or_default(),
    );
    s.total_pages = number(&c.text(&format!("{pages}/nr_hugepages")).unwrap_or_default());
    s.unix_sockets = c.text("/proc/net/unix").map_or(0, |t| t.lines().count());
    for dir in ["/data/update", "/run/poold/import"] {
        for f in c.ls(dir).unwrap_or_default() {
            if Path::new(&f)
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("part"))
                || dir.ends_with("import")
            {
                s.staging.push(format!("{dir}/{f}"));
            }
        }
    }
    let book = c.text("/data/problems/problems.json").unwrap_or_default();
    s.problems = book
        .split("\"code\":\"")
        .skip(1)
        .filter_map(|p| p.get(..8).map(str::to_string))
        .collect();
    Ok(s)
}

/// What differs between `before` and `after` that must not.
fn leaks(before: &Snapshot, after: &Snapshot, caused: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, pid, fds, rss, failures) in &before.services {
        let Some((_, pid2, fds2, rss2, failures2)) =
            after.services.iter().find(|(n, ..)| n == name)
        else {
            out.push(format!("{name} is gone"));
            continue;
        };
        if pid2 != pid || failures2 != failures {
            out.push(format!(
                "{name} restarted: process {pid} then {pid2}, {failures} then {failures2} failures"
            ));
        }
        if *fds2 > fds + 4 {
            out.push(format!("{name} holds {fds2} descriptors, {fds} before"));
        }
        if *rss2 > rss + (rss / 4).max(16 * 1024) {
            out.push(format!("{name} is {rss2} KiB resident, {rss} before"));
        }
    }
    if !after.cgroups.is_empty() {
        out.push(format!("control groups left: {:?}", after.cgroups));
    }
    let taps: Vec<&String> = after
        .links
        .iter()
        .filter(|l| l.starts_with("qsvt"))
        .collect();
    if !taps.is_empty() {
        out.push(format!("taps left: {taps:?}"));
    }
    if after.free_pages != after.total_pages {
        out.push(format!(
            "huge pages: {} of {} free with every partition ended",
            after.free_pages, after.total_pages
        ));
    }
    if after.unix_sockets > before.unix_sockets + 8 {
        out.push(format!(
            "{} Unix sockets, {} before",
            after.unix_sockets, before.unix_sockets
        ));
    }
    if !after.staging.is_empty() {
        out.push(format!("staging files left: {:?}", after.staging));
    }
    let new: Vec<&String> = after
        .problems
        .iter()
        .filter(|p| !before.problems.contains(p) && !caused.contains(&p.as_str()))
        .collect();
    if !new.is_empty() {
        out.push(format!("problems the script did not cause: {new:?}"));
    }
    out
}

/// The soak's partitions, the QS/400 one first: name and ID.
struct Parts {
    all: Vec<(String, u32)>,
}

fn activate(guest: &mut Session, id: u32, name: &str) -> Result<(), String> {
    match partd(guest)?.activate(id, name.into(), None, None) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(manager::refused(&e)),
        Err(e) => Err(e.to_string()),
    }
}

fn end(guest: &mut Session, id: u32) -> Result<(), String> {
    let _ = partd(guest)?.shut_down(id);
    manager::ended(guest, id, Instant::now() + Duration::from_mins(5)).map(|_| ())
}

/// The soak's host: the pool (the media repository), the switch, the
/// three partitions defined; and the device the image to upload is on.
fn rig<'a>(run: &Run<'a>) -> Result<(Qsi<'a>, Parts, String), String> {
    let pool = run.file("day-pool.img");
    blank(&pool, 16 << 30)?;
    let mut disks = vec![pool];
    for i in 0..2 {
        let copy = run.file(&format!("day-vs{i}.img"));
        sparse_copy(run.disk, &copy)?;
        let load = run.file(&format!("day-load{i}.img"));
        blank(&load, 512 << 20)?;
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&load)
                .map_err(|e| e.to_string())?;
            f.write_all(b"QSVS-SOAK v1\n").map_err(|e| e.to_string())?;
        }
        disks.push(copy);
        disks.push(load);
    }
    // An image to upload over and over: the installation ISO's copy.
    let iso = run
        .images
        .iso
        .clone()
        .filter(|p| p.is_file())
        .ok_or("no installation ISO to upload: `build iso --test-image`")?;
    let upload = run.file("day-upload.img");
    sparse_copy(&iso, &upload)?;
    disks.push(upload);
    let mut q = host_disks(run, true, &disks)?;
    // The QS/400 unit is QSVSUNIT0; the pool, the copies and their load
    // disks, and the image to upload follow.
    let names = disk_names(&mut q.guest, 7)?;
    crate::suite::storage::said(poold(&mut q.guest)?.create_pool(
        "DAY".into(),
        "none".into(),
        vec![names[1].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(&mut q.guest, "DAY", run.deadline(), |p| !p.mount.is_empty())?;
    crate::suite::storage::said(poold(&mut q.guest)?.designate("DAY".into()))?;
    set_switches(&mut q.guest, &[switch("DAY", 1, &[1], 1500, None)])?;
    let dev = |i: usize| format!("/dev/{}", names[i]);
    let mut parts = Parts { all: Vec::new() };
    let id = manager::partition(&mut q.guest, "DAYQS")?;
    let mut spec = Spec::new(1, crate::suite::partition::QS400_MEMORY_MIB, &dev(0));
    spec.name = "DAYQS".into();
    spec.console = model::ConsoleMode::Partition;
    spec.ethernet = vec![adapter(5, "DAY", 1, &[])];
    manager::put(&mut q.guest, id, &spec)?;
    parts.all.push(("DAYQS".into(), id));
    for i in 0..2usize {
        let name = format!("DAYVS{i}");
        let id = manager::partition(&mut q.guest, &name)?;
        let mut spec = crate::suite::partition::profile(1, 512, "side a", &dev(2 + 2 * i));
        spec.name.clone_from(&name);
        spec.backing.push(dev(3 + 2 * i));
        spec.ethernet = vec![adapter(5, "DAY", 1, &[])];
        if i == 0 {
            spec = spec.shared(50, Sharing::Uncapped);
        }
        let mut p = spec.profile();
        if i == 0 {
            p.processors.minimum_units = Some(Units(20));
            p.processors.maximum_units = Some(Units(100));
        }
        said(partd(&mut q.guest)?.put_profile(id, p))?;
        parts.all.push((name, id));
    }
    Ok((q, parts, names[6].clone()))
}

/// One more of `what` done.
fn bump(done: &mut Vec<(&'static str, u32)>, what: &'static str) {
    match done.iter_mut().find(|(w, _)| *w == what) {
        Some((_, n)) => *n += 1,
        None => done.push((what, 1)),
    }
}

/// The script's minute `tick`.
fn work(
    q: &mut Qsi<'_>,
    parts: &Parts,
    tick: u64,
    upload_device: &str,
    done: &mut Vec<(&'static str, u32)>,
) -> Result<(), String> {
    // Every partition that is not active is activated again.
    for (name, id) in &parts.all {
        if !manager::status(&mut q.guest, *id)?.state.active() {
            activate(&mut q.guest, *id, name)?;
            bump(done, "activations");
        }
    }
    let (qs, vs0, vs1) = (parts.all[0].1, parts.all[1].1, parts.all[2].1);
    if tick.is_multiple_of(10) {
        end(&mut q.guest, vs1)?;
        activate(&mut q.guest, vs1, &parts.all[2].0)?;
        bump(done, "restarts");
    }
    if tick.is_multiple_of(15) && manager::status(&mut q.guest, qs)?.state == State::Running {
        crate::suite::service::view(&mut q.guest, qs, "take", 2000, &[])?;
        bump(done, "console views");
    }
    if tick.is_multiple_of(20) {
        channel(&mut q.guest)?.ask_within(
            &format!("upload DAYIMG {upload_device}"),
            Duration::from_mins(15),
        )?;
        crate::suite::storage::said(poold(&mut q.guest)?.delete_image("DAYIMG".into()))?;
        bump(done, "uploads");
    }
    if tick.is_multiple_of(30) {
        let name = said(partd(&mut q.guest)?.take_dump(vs1))?;
        let deadline = Instant::now() + Duration::from_mins(10);
        while !said(partd(&mut q.guest)?.dumps(vs1))?
            .iter()
            .any(|d| d.name == name)
        {
            if Instant::now() >= deadline {
                return Err(format!("dump {name} was not written"));
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        said(partd(&mut q.guest)?.delete_dump(vs1, name))?;
        bump(done, "dumps");
    }
    if tick.is_multiple_of(25) {
        let units = if (tick / 25).is_multiple_of(2) {
            100
        } else {
            50
        };
        let change = Entitlement {
            units: Some(Units(units)),
            sharing: None,
            weight: None,
        };
        said(partd(&mut q.guest)?.entitlement(vs0, change, "soak".into()))?;
        bump(done, "entitlement changes");
    }
    if tick.is_multiple_of(60) {
        let (_, _, mut link) = paired(q, "MC-SOAK", Instant::now() + Duration::from_mins(3))?;
        link.close();
        crate::suite::qsi::remove_console(q, "MC-SOAK", Instant::now() + Duration::from_mins(3))?;
        bump(done, "pairings");
    }
    if tick.is_multiple_of(120) {
        end(&mut q.guest, qs)?;
        bump(done, "QS/400 shut downs");
    }
    Ok(())
}

/// The suite.
pub fn day(run: &Run<'_>) -> Result<String, String> {
    let minutes = knob("QSVS_SOAK_MINUTES", 24 * 60);
    let (mut q, parts, upload_device) = rig(run)?;
    for (name, id) in &parts.all {
        activate(&mut q.guest, *id, name)?;
    }
    std::thread::sleep(Duration::from_mins(4));
    let before = snapshot(&mut q.guest)?;
    let started = Instant::now();
    let stop = started + Duration::from_mins(minutes);
    let mut done = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    let mut tick = 0u64;
    while Instant::now() < stop && failures.len() <= 20 {
        tick += 1;
        if let Err(e) = work(&mut q, &parts, tick, &upload_device, &mut done) {
            failures.push(format!("minute {tick}: {e}"));
        }
        std::thread::sleep(Duration::from_mins(1));
    }
    for (_, id) in &parts.all {
        end(&mut q.guest, *id)?;
    }
    std::thread::sleep(Duration::from_secs(30));
    let after = snapshot(&mut q.guest)?;
    // The script's own: a pairing's attempt, a VMM ended at once.
    let caused = ["A2010030", "A2005140", "A2005150"];
    let leaked = leaks(&before, &after, &caused);
    let mut said_done = String::new();
    for (what, n) in &done {
        let _ = write!(said_done, "{n} {what}, ");
    }
    let hours = started.elapsed().as_secs_f64() / 3600.0;
    if !failures.is_empty() || !leaked.is_empty() {
        return Err(format!(
            "after {hours:.1} h: {said_done}failures {failures:?}; left behind {leaked:?}"
        ));
    }
    Ok(format!(
        "{hours:.1} h: {said_done}no service restarted, no problem the script did not cause, \
         nothing left behind ({} services, {} huge pages free of {})",
        after.services.len(),
        after.free_pages,
        after.total_pages
    ))
}
