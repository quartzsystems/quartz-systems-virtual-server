// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A confined VMM (`vmm/qsvmm`'s `confine`, `docs/SECURITY.md`).
//!
//! Two copies of the test image as partitions. The running VMM of the
//! one, as the host's kernel shows it: a seccomp filter, no new
//! privileges, no capability, the partition's own user, a network with
//! nothing but its loopback, one empty file system for its root. Then
//! the other's VMM is made, by the test hooks' `try` on its control
//! socket, to make each call a VMM never needs, from its control thread
//! and from a processor: each time it ends at once with `A2005190` and
//! its problem, while the first partition runs on in the VMM it had and
//! every service of the host stays ready.

use std::time::{Duration, Instant};

use partd::State;
use qsvshook::proto::command;

use crate::qemu::image::sparse_copy;
use crate::qemu::session::Session;
use crate::suite::base::{Run, all_ready, channel, posted};
use crate::suite::partition::manager::{self, partd};
use crate::suite::partition::{QSVS_MEMORY_MIB, ask, fresh_host, host, launch, pid_of, profile};

/// The calls tried, as `qsvmm`'s hooks name them.
const CALLS: &[&str] = &[
    "open", "socket", "exec", "fork", "ptrace", "setuid", "mount", "kill", "unshare", "ioctl",
    "bpf", "chroot",
];

/// The partition `name` on the host's disk `unit`, activated; its ID,
/// once its system has said what it was given.
fn up(guest: &mut Session, name: &str, unit: &str, run: &Run<'_>) -> Result<u32, String> {
    let id = manager::partition(guest, name)?;
    let mut spec = profile(1, QSVS_MEMORY_MIB, "side a", unit);
    spec.name = name.into();
    manager::activate(guest, id, &spec).map_err(|e| manager::refused(&e))?;
    guest.seen(
        &format!("{name}| PIO DONE"),
        run.deadline() + Duration::from_mins(3),
    )?;
    Ok(id)
}

/// A field of `/proc/<pid>/status`.
fn field<'a>(status: &'a str, name: &str) -> Result<&'a str, String> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .ok_or_else(|| format!("no {name} in the status"))
}

/// What the host's kernel shows of the running VMM `pid` of partition
/// `id`: its confinement, or what is wrong with it.
fn seen_confined(guest: &mut Session, pid: u32, id: u32) -> Result<String, String> {
    let c = channel(guest)?;
    let status = c.text(&format!("/proc/{pid}/status"))?;
    let user = (23_000 + id).to_string();
    let want = [
        ("Seccomp", "2"),
        ("NoNewPrivs", "1"),
        ("CapInh", "0000000000000000"),
        ("CapPrm", "0000000000000000"),
        ("CapEff", "0000000000000000"),
        ("CapBnd", "0000000000000000"),
        ("CapAmb", "0000000000000000"),
    ];
    for (name, value) in want {
        let got = field(&status, name)?;
        if got != value {
            return Err(format!("the VMM's {name} is {got}, not {value}"));
        }
    }
    let uid = field(&status, "Uid")?;
    if uid.split_whitespace().any(|u| u != user) {
        return Err(format!("the VMM runs as {uid}, not {user} alone"));
    }
    let filters = field(&status, "Seccomp_filters")?;
    // The process's, the processors' or the main thread's own.
    let net = c.text(&format!("/proc/{pid}/net/dev"))?;
    let links: Vec<&str> = net
        .lines()
        .skip(2)
        .filter_map(|l| l.split(':').next())
        .map(str::trim)
        .collect();
    if links != ["lo"] {
        return Err(format!("the VMM's network has {links:?}"));
    }
    let mounts = c.text(&format!("/proc/{pid}/mounts"))?;
    let mounted: Vec<&str> = mounts.lines().collect();
    // The empty root, read-only, and its own log directory bound in it.
    let log_bound = mounted.get(1).is_some_and(|m| {
        m.split_whitespace().nth(1) == Some("/log")
            && m.split_whitespace()
                .nth(3)
                .is_some_and(|o| ["nosuid", "nodev", "noexec"].iter().all(|f| o.contains(f)))
    });
    if mounted.len() != 2 || !mounted[0].starts_with("none / tmpfs ro,") || !log_bound {
        return Err(format!("the VMM's mounts are {mounted:?}"));
    }
    let root = c.ls(&format!("/proc/{pid}/root"))?;
    if root != ["log"] {
        return Err(format!("the VMM's root holds {root:?}"));
    }
    // A name resolved from its log directory stays in its root.
    let above = c.ls(&format!("/proc/{pid}/root/log/.."))?;
    if above != ["log"] {
        return Err(format!("above the VMM's log directory is {above:?}"));
    }
    let log = c.ls(&format!("/proc/{pid}/root/log"))?;
    if !log.iter().any(|f| f.starts_with("serial")) {
        return Err(format!("the VMM's log directory holds {log:?}"));
    }
    Ok(format!(
        "seccomp 2 with {filters} filter(s), no new privileges, no capability, user {user}, \
         network `lo` alone, a read-only tmpfs root holding nothing but its own log \
         directory, bound there with nosuid, nodev, noexec, above which is that root"
    ))
}

/// The suite.
pub fn confined(run: &Run<'_>) -> Result<String, String> {
    fresh_host(run)?;
    let a = run.file("unit-a.img");
    let b = run.file("unit-b.img");
    sparse_copy(run.disk, &a)?;
    sparse_copy(run.disk, &b)?;
    let mut guest = host(run, &launch(run, &[a, b])?)?;
    let calm = up(&mut guest, "CALM", "/dev/vdb", run)?;
    let calm_pid = pid_of(&mut guest, calm)?;
    let seen = seen_confined(&mut guest, calm_pid, calm)?;
    let mut ended = 0;
    let mut said = Vec::new();
    for place in ["control", "processor"] {
        for call in CALLS {
            let tried = up(&mut guest, "TRIED", "/dev/vdc", run)?;
            // The VMM ends before it answers: what comes back is no matter.
            let answer = ask(
                &mut guest,
                &format!("{} control {tried} try {place} {call}", command::PARTITION),
            );
            let deadline = Instant::now() + Duration::from_mins(1);
            let status = manager::until(&mut guest, tried, State::Error, deadline)
                .map_err(|e| format!("{call} from the {place}: {e}; it answered {answer:?}"))?;
            if status.code != refcode::PARTITION_FORBIDDEN.code {
                return Err(format!(
                    "{call} from the {place}: `{} {}`, {}",
                    status.code, status.text, status.last_end
                ));
            }
            if status.current.is_some() {
                return Err(format!(
                    "{call} from the {place}: it still holds its resources"
                ));
            }
            // The other partition, in the VMM it had; the host whole.
            if pid_of(&mut guest, calm)? != calm_pid {
                return Err(format!("{call} from the {place}: CALM has another VMM"));
            }
            let calm_now = manager::status(&mut guest, calm)?;
            if !calm_now.state.active() {
                return Err(format!(
                    "{call} from the {place}: CALM is {}",
                    calm_now.state.word()
                ));
            }
            all_ready(&channel(&mut guest)?.status()?)?;
            ended += 1;
            if said.len() < 2 {
                said.push(format!("{call} from the {place}: {}", status.last_end));
            }
        }
    }
    guest.seen(
        &posted(&refcode::PARTITION_FORBIDDEN, "TRIED"),
        Instant::now() + Duration::from_secs(10),
    )?;
    let book = channel(&mut guest)?.text("/data/problems/problems.json")?;
    if !book.contains(refcode::PARTITION_FORBIDDEN.code) {
        return Err("no problem was opened for A2005190".into());
    }
    said_partd_answers(&mut guest)?;
    Ok(format!(
        "CALM's VMM: {seen}; TRIED's VMM ended {ended} times of {} with A2005190 ({}), its \
         problem opened; CALM ran on in VMM {calm_pid} and every service stayed ready",
        2 * CALLS.len(),
        said.join("; ")
    ))
}

/// The partition manager still answers.
fn said_partd_answers(guest: &mut Session) -> Result<(), String> {
    use partd::Calls as _;
    partd(guest)?
        .list()
        .map(|_| ())
        .map_err(|e| format!("the partition manager: {e}"))
}
