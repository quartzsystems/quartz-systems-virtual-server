// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The baselines of `docs/PHASE11.md`: numbers kept, no threshold. A
//! host of 8 GiB is timed from power on to its base ready; the cheap
//! guest and QS/400 from activation to running; a pool of each
//! protection is written and read with direct I/O; keys typed at
//! QS/400's console are timed to the screen's change; and what the host
//! itself uses, memory outside the partitions' huge pages and processor
//! time, is taken with 0, 1, 8 and 32 idle partitions. Where the host is
//! itself a virtual machine the numbers say little of a real one; they
//! are taken to compare one build with the next.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use model::Sharing;
use partd::{Calls as _, State};
use poold::proto::Calls as _;

use crate::qemu::image::{blank, sparse_copy};
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel, posted};
use crate::suite::network::{Rig, guest_ready, guest_spec, rig_launch};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::partition::{QS400_MEMORY_MIB, fresh_host, host, launch, qs400_image};
use crate::suite::storage::{POOL_DISK_BYTES, disk_names, pool_until, poold};

/// The host's memory: room for 32 idle partitions and QS/400.
const HOST_MIB: u32 = 8192;
/// An idle partition's memory.
const IDLE_MIB: u64 = 128;
/// How much of each pool is written and read.
const BENCH_MIB: u32 = 512;
/// How long the host is left before it is measured, and measured for.
const SETTLE: Duration = Duration::from_secs(30);
const MEASURE: Duration = Duration::from_secs(20);

/// The pools: name, protection, members (indexes into the pool disks).
const POOLS: [(&str, &str, &[usize]); 4] = [
    ("NONE", "none", &[0]),
    ("MIRR", "mirrored", &[1, 2]),
    ("RAID5", "raid5", &[3, 4, 5]),
    ("RAID6", "raid6", &[6, 7, 8, 9]),
];

/// The value of `key` in `/proc/meminfo`, in KiB.
fn meminfo(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// The processors' total and idle time, from `/proc/stat`'s first line.
fn ticks(g: &mut Session) -> Result<(u64, u64), String> {
    let text = channel(g)?.text("/proc/stat")?;
    let all: Vec<u64> = text
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .skip(1)
        .filter_map(|v| v.parse().ok())
        .collect();
    // idle and iowait are the fourth and fifth.
    let idle = all.get(3).copied().unwrap_or(0) + all.get(4).copied().unwrap_or(0);
    Ok((all.iter().sum(), idle))
}

/// What the host uses with `n` idle partitions: MiB outside the huge
/// pages, and the share of its processors that were busy.
fn host_use(g: &mut Session, n: usize) -> Result<String, String> {
    std::thread::sleep(SETTLE);
    let (total0, idle0) = ticks(g)?;
    std::thread::sleep(MEASURE);
    let (total1, idle1) = ticks(g)?;
    // Tenths of a percent, in whole numbers.
    let idle = u32::try_from((idle1 - idle0) * 1000 / (total1 - total0).max(1)).unwrap_or(1000);
    let busy = f64::from(1000u32.saturating_sub(idle)) / 10.0;
    let m = channel(g)?.text("/proc/meminfo")?;
    let used = meminfo(&m, "MemTotal")
        .saturating_sub(meminfo(&m, "MemAvailable"))
        .saturating_sub(meminfo(&m, "Hugetlb"));
    Ok(format!(
        "{n} idle: {} MiB used outside huge pages, processors {busy:.1}% busy",
        used / 1024
    ))
}

/// Seconds since `t`.
fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

/// The host's disks: the halting guest's disk, QS/400's copy, ten pool
/// disks.
fn disks(run: &Run<'_>) -> Result<Vec<PathBuf>, String> {
    let halter = run.file("halter.img");
    let image = ipl::build_disk(&[(ipl::SIDE_A_PATH, &ipl::load::halter())], true);
    std::fs::write(&halter, image).map_err(|e| format!("{}: {e}", halter.display()))?;
    let qs400 = run.file("qs400.img");
    sparse_copy(&qs400_image()?, &qs400)?;
    let mut all = vec![halter, qs400];
    for i in 0..10 {
        let p = run.file(&format!("bench{i}.img"));
        blank(&p, POOL_DISK_BYTES)?;
        all.push(p);
    }
    Ok(all)
}

/// QS/400 activated and running: the time it took, and the console's
/// latency for ten keys typed at it.
fn qs400(g: &mut Session, device: &str, run: &Run<'_>) -> Result<String, String> {
    let id = manager::partition(g, "BENCHQS")?;
    let mut spec = Spec::new(1, QS400_MEMORY_MIB, device);
    spec.name = "BENCHQS".into();
    spec.console = model::ConsoleMode::Partition;
    let started = Instant::now();
    manager::activate(g, id, &spec).map_err(|e| manager::refused(&e))?;
    manager::until(g, id, State::Running, run.deadline())?;
    let ipl = secs(started);
    // The console's first screen is drawn by then.
    std::thread::sleep(Duration::from_secs(20));
    let seen = crate::suite::service::view(
        g,
        id,
        "take",
        3000,
        &crate::suite::service::typed("QSECOFRXYZ"),
    )?;
    let latency = seen.fact("latency_ms").to_string();
    said(partd(g)?.shut_down_as(id, partd::proto::Shutdown::Immediate, 0, false))?;
    manager::ended(g, id, Instant::now() + Duration::from_mins(2))?;
    Ok(format!(
        "QS/400 IPL (activation to running): {ipl:.1} s; console, ms from a key to the \
         screen's change: {latency}"
    ))
}

/// Each pool made, its work done, written and read.
fn pools(g: &mut Session, names: &[String], run: &Run<'_>) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for (name, protection, members) in POOLS {
        let disks = members.iter().map(|i| names[*i].clone()).collect();
        crate::suite::storage::said(poold(g)?.create_pool(
            name.into(),
            protection.into(),
            disks,
            Vec::new(),
            Vec::new(),
        ))?;
        let made = Instant::now();
        let p = pool_until(g, name, run.deadline(), |p| {
            !p.mount.is_empty() && p.work.is_empty()
        })?;
        let ready = secs(made);
        let answer = channel(g)?.ask_within(
            &format!("bench {}/bench.dat {BENCH_MIB}", p.mount),
            Duration::from_mins(10),
        )?;
        let words: Vec<&str> = answer
            .first()
            .map_or(Vec::new(), |l| l.split(' ').collect());
        let [_, write, read] = words.as_slice() else {
            return Err(format!("bench in {name}: {answer:?}"));
        };
        lines.push(format!(
            "pool {name} ({protection}, {} disks, ready in {ready:.0} s): write {write} MB/s, \
             read {read} MB/s",
            members.len()
        ));
    }
    Ok(lines)
}

/// The suite.
pub fn baselines(run: &Run<'_>) -> Result<String, String> {
    fresh_host(run)?;
    let mut rig = Rig::new(0, 1);
    rig.disks = disks(run)?;
    let mut base = launch(run, &[])?;
    base.memory_mb = HOST_MIB;
    let (l, _station) = rig_launch(run, &rig, base)?;
    let started = Instant::now();
    let mut guest = host(run, &l)?;
    let g = &mut guest;
    g.seen(&posted(&refcode::BASE_READY, ""), run.deadline())?;
    let mut report = vec![format!(
        "host: power on to base ready {:.1} s",
        secs(started)
    )];

    // The host's seed, the guest's disk and seed, the halting guest's,
    // QS/400's, then the ten pool disks.
    let names = disk_names(g, 15)?;
    let id = manager::partition(g, "BENCHVS")?;
    let started = Instant::now();
    manager::activate(g, id, &guest_spec(0, Vec::new())).map_err(|e| manager::refused(&e))?;
    guest_ready(g, "BENCHVS", run.deadline())?;
    report.push(format!(
        "cheap guest: activation to its base ready {:.1} s",
        secs(started)
    ));
    said(partd(g)?.shut_down_as(id, partd::proto::Shutdown::Immediate, 0, false))?;
    manager::ended(g, id, Instant::now() + Duration::from_mins(2))?;

    report.push(qs400(g, &format!("/dev/{}", names[4]), run)?);
    report.extend(pools(g, &names[5..], run)?);

    // Idle partitions, each on a copy of the halting guest in NONE.
    let image_bytes = std::fs::metadata(&rig.disks[0])
        .map_err(|e| e.to_string())?
        .len();
    let mount = said_pool_mount(g)?;
    report.push(host_use(g, 0)?);
    let mut ids = Vec::new();
    for n in [1usize, 8, 32] {
        while ids.len() < n {
            let i = ids.len();
            let name = format!("IDLE{i:02}");
            crate::suite::storage::said(poold(g)?.create_vdisk(
                "NONE".into(),
                name.clone(),
                64,
                false,
            ))?;
            channel(g)?.ask(&format!(
                "copy /dev/{} 0 {}/{}/{name}{} 0 {image_bytes}",
                names[3],
                mount,
                vdisk::DIR,
                vdisk::SUFFIX
            ))?;
            let id = manager::partition(g, &name)?;
            let mut spec =
                Spec::new(1, IDLE_MIB, &format!("NONE/{name}")).shared(10, Sharing::Capped);
            spec.name = "IDLE".into();
            manager::activate(g, id, &spec).map_err(|e| manager::refused(&e))?;
            ids.push(id);
        }
        for id in &ids {
            manager::until(g, *id, State::Running, run.deadline())?;
        }
        report.push(host_use(g, n)?);
    }
    let mut text = String::new();
    for line in &report {
        let _ = writeln!(text, "{line}");
    }
    Ok(text.trim_end().replace('\n', "; "))
}

/// Where the pool NONE is mounted.
fn said_pool_mount(g: &mut Session) -> Result<String, String> {
    poold(g)?
        .pools()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.name == "NONE")
        .map(|p| p.mount)
        .ok_or_else(|| "no pool NONE".into())
}
