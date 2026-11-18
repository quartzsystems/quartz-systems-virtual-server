// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Section 1's soak (`docs/PHASE11.md`): partitions started over and over
//! under load, three levels deep, until the soak's time is up.
//!
//! The host has a pool for dumps, a scratch disk to copy them out, an
//! internal switch, and partitions of two kinds: copies of the test
//! image, each with a disk marked `QSVS-SOAK` (its system runs the
//! soak's load, `qsvshook soak`), and copies of QS/400's image. Each IPL
//! runs for `QSVS_SOAK_ROUND` seconds after its system is up and is shut
//! down; a line of a partition's that names a fault, or a partition held
//! in Error, is a fault: the partition is dumped, the dump copied out
//! and read ([`super::core`]), and the partition started again.
//!
//! The VMM's test hooks hold a guest that restarts (its kernel
//! panicked), and with `QSVS_SOAK_HOOKS` take the controls' words
//! (`smallpages`, `norng`, `noconsole`). The other knobs:
//! `QSVS_SOAK_MINUTES`, `QSVS_SOAK_QSVS` and `QSVS_SOAK_QS400` (how many
//! of each kind), `QSVS_SOAK_VCPUS`, `QSVS_SOAK_MIB` (a test image
//! partition's memory), `QSVS_SOAK_NET` (0: no adapters),
//! `QSVS_SOAK_OUT` (the report's directory); `QSVS_NO_IOMMU` as for
//! every suite.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use model::profile::IplSource;
use partd::{Calls as _, State};
use poold::proto::Calls as _;
use qsvshook::partition::relayed;
use qsvshook::proto::command;

use super::core::records;
use super::report::{Report, Round};
use super::{knob, length, words};
use crate::qemu::image::{blank, sparse_copy};
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel};
use crate::suite::network::{adapter, set_switches, switch};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::partition::{QS400_MEMORY_MIB, host, launch, profile, qs400_image};
use crate::suite::storage::{disk_names, pool_until, poold};

/// What marks a line of a partition's as a fault.
pub const FAULT: &[&str] = &[
    "BUG: ",
    "Oops",
    "general protection",
    "Kernel panic",
    "double fault",
    "stack-protector",
    "SOAK BAD",
    "Bad rss-counter",
    "kernel BUG",
    "invalid opcode",
    "nable to handle",
    "soft lockup",
    "hard LOCKUP",
    "rcu_sched self-detected",
    "rcu: INFO",
];

/// The file the VMM reads its test hooks from.
const VMM_HOOKS: &str = "/run/qsvshook.vmm";
/// The switch the partitions share.
const SWITCH: &str = "SOAK";
/// How long a system may take to be up: QS/400 three levels deep is slow.
const QS400_UP: Duration = Duration::from_mins(15);
const QSVS_UP: Duration = Duration::from_mins(6);
/// How long a partition that said a fault is given to be held by its
/// panic before it is dumped as it runs.
const HOLD_WAIT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Qsvs,
    Qs400,
}

/// One partition of the soak and its IPL in progress.
struct Soaked {
    name: String,
    kind: Kind,
    id: u32,
    spec: Spec,
    ipl: u64,
    began: Option<Instant>,
    up: Option<Instant>,
    faults: Vec<String>,
    fault_at: Option<Instant>,
    oks: u64,
}

impl Soaked {
    fn round(&self, held: bool, dump: Option<String>, end: Option<String>) -> Round {
        Round {
            partition: self.name.clone(),
            kind: match self.kind {
                Kind::Qsvs => "qsvs",
                Kind::Qs400 => "qs400",
            }
            .into(),
            ipl: self.ipl,
            up_secs: match (self.began, self.up) {
                (Some(b), Some(u)) => Some(u.duration_since(b).as_secs_f64()),
                _ => None,
            },
            secs: self.began.map_or(0.0, |b| b.elapsed().as_secs_f64()),
            faults: self.faults.clone(),
            held,
            dump,
            end,
        }
    }

    fn reset(&mut self) {
        self.began = None;
        self.up = None;
        self.faults.clear();
        self.fault_at = None;
        self.oks = 0;
    }
}

/// The disks: the pool's, the scratch disk, then each partition's.
struct Disks {
    units: Vec<PathBuf>,
    scratch: PathBuf,
}

fn device(i: usize) -> String {
    let letter = char::from(b'b' + u8::try_from(i).unwrap_or(0));
    format!("/dev/vd{letter}")
}

/// The soak's partitions and their disks.
fn plan(run: &Run<'_>) -> Result<(Disks, Vec<Soaked>), String> {
    let qsvs = knob("QSVS_SOAK_QSVS", 2);
    let qs400 = knob("QSVS_SOAK_QS400", 1);
    let vcpus = u32::try_from(knob("QSVS_SOAK_VCPUS", 2)).unwrap_or(2);
    let mib = knob("QSVS_SOAK_MIB", 1024);
    let net = knob("QSVS_SOAK_NET", 1) != 0;
    let pool = run.file("soak-pool.img");
    blank(&pool, 8 << 30)?;
    let scratch = run.file("soak-scratch.img");
    blank(&scratch, 4 << 30)?;
    let mut units = vec![pool, scratch.clone()];
    let mut soaked = Vec::new();
    let adapters = |spec: &mut Spec| {
        if net {
            spec.ethernet = vec![adapter(5, SWITCH, 1, &[])];
        }
    };
    for i in 0..qs400 {
        let copy = run.file(&format!("soak-qs400-{i}.img"));
        sparse_copy(&qs400_image()?, &copy)?;
        units.push(copy);
        let mut spec = Spec::new(1, QS400_MEMORY_MIB, &device(units.len() - 1));
        spec.console = model::ConsoleMode::Partition;
        spec.keylock = Some(model::Keylock::Normal);
        adapters(&mut spec);
        soaked.push(Soaked {
            name: format!("QSOAK{i}"),
            kind: Kind::Qs400,
            id: 0,
            spec,
            ipl: 0,
            began: None,
            up: None,
            faults: Vec::new(),
            fault_at: None,
            oks: 0,
        });
    }
    for i in 0..qsvs {
        let copy = run.file(&format!("soak-qsvs-{i}.img"));
        sparse_copy(run.disk, &copy)?;
        units.push(copy);
        let disk = units.len() - 1;
        let marked = run.file(&format!("soak-load-{i}.img"));
        blank(&marked, 512 << 20)?;
        mark(&marked)?;
        units.push(marked);
        let mut spec = profile(vcpus, mib, "side a", &device(disk));
        spec.side = IplSource::A;
        spec.backing.push(device(disk + 1));
        adapters(&mut spec);
        soaked.push(Soaked {
            name: format!("VSOAK{i}"),
            kind: Kind::Qsvs,
            id: 0,
            spec,
            ipl: 0,
            began: None,
            up: None,
            faults: Vec::new(),
            fault_at: None,
            oks: 0,
        });
    }
    for s in &mut soaked {
        s.spec.name = s.name.clone();
    }
    Ok((Disks { units, scratch }, soaked))
}

/// The soak's marker at the start of a disk.
fn mark(path: &PathBuf) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(b"QSVS-SOAK v1\n").map_err(|e| e.to_string())
}

/// The pool for dumps, designated the media repository: its mount.
fn repository(guest: &mut Session, run: &Run<'_>) -> Result<String, String> {
    let names = disk_names(guest, 1)?;
    crate::suite::storage::said(poold(guest)?.create_pool(
        "SOAKPOOL".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    let p = pool_until(guest, "SOAKPOOL", run.deadline(), |p| !p.mount.is_empty())?;
    crate::suite::storage::said(poold(guest)?.designate("SOAKPOOL".into()))?;
    Ok(p.mount)
}

/// The settings of this soak, in a line.
fn settings(soaked: &[Soaked]) -> String {
    let qs400 = soaked.iter().filter(|s| s.kind == Kind::Qs400).count();
    format!(
        "{} test image and {qs400} QS/400 partition(s), {} processor(s) and {} MiB each \
         (test image), network {}, IOMMU {}, hooks `{}`",
        soaked.len() - qs400,
        knob("QSVS_SOAK_VCPUS", 2),
        knob("QSVS_SOAK_MIB", 1024),
        if knob("QSVS_SOAK_NET", 1) == 0 {
            "off"
        } else {
            "on"
        },
        if crate::qemu::iommu_wanted() {
            "on"
        } else {
            "off"
        },
        words("QSVS_SOAK_HOOKS").trim()
    )
}

/// The soak.
pub fn faults(run: &Run<'_>) -> Result<String, String> {
    let (disks, mut soaked) = plan(run)?;
    crate::suite::partition::fresh_host(run)?;
    let mut l = launch(run, &disks.units)?;
    let guests_mib: u64 = soaked
        .iter()
        .map(|s| match s.kind {
            Kind::Qs400 => QS400_MEMORY_MIB,
            Kind::Qsvs => knob("QSVS_SOAK_MIB", 1024),
        })
        .sum();
    // The host's own two GiB beside the partitions' huge pages.
    l.memory_mb = u32::try_from(guests_mib + 2560).unwrap_or(l.memory_mb);
    let out =
        std::env::var_os("QSVS_SOAK_OUT").map_or_else(|| run.file("soak-report"), PathBuf::from);
    let mut report = Report::new(&out)?;
    let line = settings(&soaked);
    std::fs::write(report.file("settings.txt"), format!("{line}\n")).map_err(|e| e.to_string())?;
    let mut guest = host(run, &l)?;
    let mount = repository(&mut guest, run)?;
    let scratch_device = format!("/dev/{}", disk_names(&mut guest, 2)?[1]);
    let hooks = format!("holdrestart {}\n", words("QSVS_SOAK_HOOKS"));
    channel(&mut guest)?.ask(&format!(
        "{} {VMM_HOOKS} {}",
        command::PUT,
        hex::encode(hooks.as_bytes())
    ))?;
    if knob("QSVS_SOAK_NET", 1) != 0 {
        set_switches(&mut guest, &[switch(SWITCH, 1, &[1], 1500, None)])?;
    }
    for s in &mut soaked {
        s.id = manager::partition(&mut guest, &s.name)?;
        manager::put(&mut guest, s.id, &s.spec)?;
    }
    let started = Instant::now();
    let stop = started + length();
    let round = Duration::from_secs(knob("QSVS_SOAK_ROUND", 180));
    let mut scanned = 0usize;
    while Instant::now() < stop {
        scanned = scan(&mut guest, &mut soaked, scanned);
        for s in &mut soaked {
            step(
                &mut guest,
                s,
                round,
                &mut report,
                &mount,
                &scratch_device,
                &disks,
            )?;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    for s in &mut soaked {
        let _ = said(partd(&mut guest)?.shut_down(s.id));
        let _ = manager::ended(&mut guest, s.id, Instant::now() + Duration::from_mins(2));
        if s.began.is_some() {
            let r = s.round(false, None, Some("the soak's end".into()));
            if r.up_secs.is_some() || !r.faults.is_empty() {
                report.add(&r);
            }
        }
    }
    let summary = report.summary(started.elapsed(), &line);
    if report.up == 0 {
        return Err(format!("no partition came up: {summary}"));
    }
    Ok(format!("{summary}; report in {}", out.display()))
}

/// Read what the partitions said since `from`: faults and the soak's
/// lines. Only whole lines are read; where the reading stopped.
fn scan(guest: &mut Session, soaked: &mut [Soaked], from: usize) -> usize {
    guest.drain();
    let log = &guest.log;
    let end = log[from.min(log.len())..]
        .rfind('\n')
        .map_or(from, |i| from + i + 1);
    for line in log[from.min(end)..end].lines() {
        for s in soaked.iter_mut() {
            let mark = relayed(&s.name, "");
            let Some(text) = line.find(&mark).map(|i| &line[i + mark.len()..]) else {
                continue;
            };
            if s.began.is_none() {
                continue;
            }
            if text.contains("SOAK ok") {
                s.oks += 1;
            }
            if FAULT.iter().any(|f| text.contains(f)) && s.faults.len() < 60 {
                s.faults.push(text.trim_end().to_string());
                s.fault_at.get_or_insert_with(Instant::now);
            }
        }
    }
    end
}

/// One look at a partition: start it, see it up, end its round, or
/// deal with its fault.
fn step(
    guest: &mut Session,
    s: &mut Soaked,
    round: Duration,
    report: &mut Report,
    mount: &str,
    scratch_device: &str,
    disks: &Disks,
) -> Result<(), String> {
    let status = manager::status(guest, s.id)?;
    let Some(began) = s.began else {
        if !status.state.active() {
            s.reset();
            s.ipl += 1;
            s.began = Some(Instant::now());
            if let Err(e) = partd(guest)?.activate(s.id, s.name.clone(), None, s.spec.keylock) {
                return Err(format!("{}: activation: {e}", s.name));
            }
        }
        return Ok(());
    };
    if s.up.is_none() {
        let up = match s.kind {
            Kind::Qsvs => s.oks > 0,
            Kind::Qs400 => status.state == State::Running,
        };
        if up {
            s.up = Some(Instant::now());
        }
    }
    let held = status.state == State::Error && status.current.is_some();
    let ended = !status.state.active();
    if held || ended || s.fault_at.is_some_and(|t| t.elapsed() > HOLD_WAIT) {
        // A fault: held by its panic, ended, or running on after it.
        let dump = if status.state.active() {
            Some(dump(guest, s, report, mount, scratch_device, disks)?)
        } else {
            None
        };
        let end = format!(
            "{} `{} {}` {}",
            status.state.word(),
            status.code,
            status.text,
            status.last_end
        );
        end_it(guest, s)?;
        report.add(&s.round(held, dump, Some(end)));
        s.reset();
        return Ok(());
    }
    if s.fault_at.is_some() {
        // Its panic may yet hold it: wait.
        return Ok(());
    }
    let patience = match s.kind {
        Kind::Qs400 => QS400_UP,
        Kind::Qsvs => QSVS_UP,
    };
    let done = match s.up {
        Some(up) => up.elapsed() >= round,
        None => began.elapsed() >= patience,
    };
    if done {
        end_it(guest, s)?;
        let end = s.up.is_none().then(|| {
            format!(
                "never up: {} `{} {}`",
                status.state.word(),
                status.code,
                status.text
            )
        });
        report.add(&s.round(false, None, end));
        s.reset();
    }
    Ok(())
}

/// Shut the partition down at once and wait for its end.
fn end_it(guest: &mut Session, s: &Soaked) -> Result<(), String> {
    let _ = partd(guest)?.shut_down(s.id);
    manager::ended(guest, s.id, Instant::now() + Duration::from_mins(3)).map(|_| ())
}

/// Dump the partition, copy the dump out and read it; the copy's path.
fn dump(
    guest: &mut Session,
    s: &Soaked,
    report: &Report,
    mount: &str,
    scratch_device: &str,
    disks: &Disks,
) -> Result<String, String> {
    let name = match said(partd(guest)?.take_dump(s.id)) {
        Ok(name) => name,
        Err(e) => return Ok(format!("not taken: {e}")),
    };
    let deadline = Instant::now() + Duration::from_mins(10);
    let info = loop {
        // Listed once it is whole.
        let found = said(partd(guest)?.dumps(s.id))?
            .into_iter()
            .find(|d| d.name == name);
        if let Some(d) = found {
            break d;
        }
        if Instant::now() >= deadline {
            return Ok(format!("{name} not written in time"));
        }
        std::thread::sleep(Duration::from_secs(2));
    };
    let from = format!("{mount}/dumps/{}/{name}", s.id);
    let to = report.dump_path(&s.name, s.ipl);
    crate::suite::service::ask(
        guest,
        &format!("copy {from} 0 {scratch_device} 0 {}", info.bytes),
    )?;
    {
        let mut src = std::fs::File::open(&disks.scratch).map_err(|e| e.to_string())?;
        let mut dst = std::fs::File::create(&to).map_err(|e| e.to_string())?;
        std::io::copy(&mut std::io::Read::take(&mut src, info.bytes), &mut dst)
            .map_err(|e| e.to_string())?;
    }
    let _ = said(partd(guest)?.delete_dump(s.id, name));
    let read = records(&to).map_err(|e| e.to_string())?;
    let text = read.iter().fold(String::new(), |mut text, (at, t)| {
        let _ = writeln!(text, "{at:#012x} {t}");
        text
    });
    let mut notes = to.clone().into_os_string();
    notes.push(".txt");
    std::fs::write(&notes, text).map_err(|e| e.to_string())?;
    Ok(to.display().to_string())
}
