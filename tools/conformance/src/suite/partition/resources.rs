// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Processors and memory. The guests here do nothing but run
//! (`ipl::load::spinner`): each has one busy processor from its first
//! instruction, which is what a measurement of processor time wants,
//! and is small enough for several to run in a host under the harness.
//!
//! What is measured is the kernel's own account: a control group's
//! `cpu.stat`, a thread's allowed processors and the processor it last
//! ran on, what each processor runs now, the huge pages a mapping is
//! made of.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use model::Units;
use model::profile::Sharing;
use partd::{Calls as _, State};
use qsvshook::proto::{command, word};
use resource::{Pool, Processors};

use super::manager::{self, Spec, partd, said};
use super::{ask, expect_code, fresh_host, host, host_unaffected, launch, pid_of};
use crate::qemu::Launch;
use crate::qemu::session::Session;
use crate::suite::base::{Guest, Run, channel, posted};

/// A spinner's memory: the least a partition has.
pub const SPIN_MIB: u64 = 128;
/// How long processor time is measured for.
const MEASURE: Duration = Duration::from_secs(20);
/// How far a measured share may be from what was asked, where the
/// host is itself a virtual machine: the handoff allows 10% there and
/// 5% on a machine that is none.
const TOLERANCE: f64 = 0.10;

/// The device of the `i`th further disk of the host, from 0.
pub fn device(i: usize) -> String {
    let letter = char::from(b'b' + u8::try_from(i).unwrap_or(0));
    format!("/dev/vd{letter}")
}

/// The name of the `i`th partition, from 0.
pub fn name(i: usize) -> String {
    format!("P{}", i + 1)
}

/// A disk whose side A is the spinner.
fn spinner_disk(run: &Run<'_>, i: usize) -> Result<PathBuf, String> {
    let path = run.file(&format!("spin{i}.img"));
    let disk = ipl::build_disk(&[(ipl::SIDE_A_PATH, &ipl::load::spinner())], true);
    std::fs::write(&path, disk).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The launch of a host with `count` spinner disks.
pub fn spinner_launch(run: &Run<'_>, count: usize) -> Result<Launch, String> {
    fresh_host(run)?;
    let disks: Result<Vec<PathBuf>, String> = (0..count).map(|i| spinner_disk(run, i)).collect();
    launch(run, &disks?)
}

/// Boot `launch` and define a partition for each of its `count`
/// spinner disks; their IDs. Nothing is activated.
pub fn spinners<'a>(
    run: &Run<'a>,
    launch: &Launch,
    count: usize,
) -> Result<(Guest<'a>, Vec<u32>), String> {
    let mut guest = host(run, launch)?;
    let mut ids = Vec::new();
    for i in 0..count {
        ids.push(manager::partition(&mut guest, &name(i))?);
    }
    Ok((guest, ids))
}

/// The profile of the `i`th spinner: `processors` dedicated, the
/// least memory, its IPL from side A.
pub fn spin(i: usize, processors: u32) -> Spec {
    let mut spec = Spec::new(processors, SPIN_MIB, &device(i));
    spec.side = model::IplSource::A;
    spec
}

/// Activate the partition `id` with `spec` and wait until it runs.
pub fn run_one(
    guest: &mut Session,
    id: u32,
    spec: &Spec,
    deadline: Instant,
) -> Result<resource::Current, String> {
    let current = manager::activate(guest, id, spec).map_err(|errors| manager::refused(&errors))?;
    manager::until(guest, id, State::Running, deadline)?;
    Ok(current)
}

/// The numbers in a line of the hook's answer.
fn line_words(line: &str) -> Option<Vec<&str>> {
    Some(
        line.strip_prefix(word::LINE)?
            .split_ascii_whitespace()
            .collect(),
    )
}

/// One thread of a process, as the hook's `threads` has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    /// Its id.
    pub tid: u32,
    /// Its state: `R`, `S`.
    pub state: String,
    /// The processor it last ran on.
    pub processor: u32,
    /// Its user and system ticks.
    pub ticks: u64,
    /// The processors it is allowed.
    pub allowed: Vec<u32>,
    /// Its name.
    pub name: String,
}

/// The threads of the process `pid` on the host.
pub fn threads(guest: &mut Session, pid: u32) -> Result<Vec<Thread>, String> {
    let lines = ask(guest, &format!("{} {pid}", command::THREADS))?;
    let mut threads = Vec::new();
    for line in &lines {
        let Some(words) = line_words(line) else {
            continue;
        };
        let parsed = (|| {
            Some(Thread {
                tid: words.first()?.parse().ok()?,
                state: (*words.get(1)?).to_string(),
                processor: words.get(2)?.parse().ok()?,
                ticks: words.get(3)?.parse().ok()?,
                allowed: resource::topology::parse_cpu_list(words.get(4)?)?,
                name: words.get(5..)?.join(" "),
            })
        })();
        threads.push(parsed.ok_or_else(|| format!("`threads {pid}` said `{line}`"))?);
    }
    Ok(threads)
}

/// A group's processor time so far, in microseconds, with the host's
/// uptime in seconds when it was read, and its forced idle time.
fn usage(guest: &mut Session, group: &str) -> Result<(u64, f64, u64), String> {
    let channel = channel(guest)?;
    let stat = channel.text(&format!("/sys/fs/cgroup/{group}/cpu.stat"))?;
    let uptime = channel.text("/proc/uptime")?;
    let value = |key: &str| {
        stat.lines()
            .find_map(|l| l.strip_prefix(key)?.trim().parse::<u64>().ok())
    };
    let used = value("usage_usec ").ok_or_else(|| format!("{group}: no usage in `{stat}`"))?;
    let at = uptime
        .split_ascii_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .ok_or("the host's uptime")?;
    Ok((used, at, value("core_sched.force_idle_usec ").unwrap_or(0)))
}

/// The processor time each of `groups` used over [`MEASURE`], as a
/// number of threads kept busy, and the forced idle time each had.
fn measure(guest: &mut Session, groups: &[String]) -> Result<Vec<(f64, u64)>, String> {
    let mut before = Vec::new();
    for group in groups {
        before.push(usage(guest, group)?);
    }
    std::thread::sleep(MEASURE);
    let mut out = Vec::new();
    for (group, (used, at, idle)) in groups.iter().zip(before) {
        let (now_used, now_at, now_idle) = usage(guest, group)?;
        // Microseconds of processor over seconds of wall clock.
        #[allow(clippy::cast_precision_loss)]
        let threads = (now_used - used) as f64 / 1e6 / (now_at - at);
        out.push((threads, now_idle - idle));
    }
    Ok(out)
}

/// Whether a name in a control group's directory is a partition's
/// group, `p<ID>`, and not one of the kernel's files.
fn is_partition_group(name: &str) -> bool {
    name.strip_prefix('p')
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Three partitions activate and run together; a fourth whose minimum
/// memory is not available is refused with the right message and
/// nothing is left behind.
pub fn three(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 4)?, 4)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let before = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let mut pids = Vec::new();
    for (i, id) in ids.iter().take(3).enumerate() {
        run_one(&mut guest, *id, &spin(i, 1), deadline)?;
        pids.push(pid_of(&mut guest, *id)?);
    }
    let all = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
    let running = all.iter().filter(|p| p.state == State::Running).count();
    if running != 3 || pids[0] == pids[1] || pids[1] == pids[2] {
        return Err(format!(
            "{running} of three partitions run, their VMMs {pids:?}"
        ));
    }
    let with_three = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    if with_three.memory.available != before.memory.available - 3 * SPIN_MIB
        || with_three.processors.available != before.processors.available - 300
    {
        return Err(format!(
            "three partitions of a core and {SPIN_MIB} MiB each: {before:?} before, \
             {with_three:?} with them"
        ));
    }

    // The fourth wants more memory than is left, at the least.
    let mut wants = spin(3, 1);
    let too_much = with_three.memory.available + before.memory_block_mib;
    wants.memory_mib = (too_much, too_much);
    let refused = manager::activate(&mut guest, ids[3], &wants)
        .err()
        .ok_or("a partition whose minimum memory is not there was activated")?;
    let said_so = format!(
        "Partition {} needs {too_much} MB of memory; {} MB are available.",
        name(3),
        with_three.memory.available
    );
    if refused.len() != 1
        || !refused[0].is(&qvsmsg::MEMORY_NOT_AVAILABLE)
        || refused[0].text != said_so
    {
        return Err(format!(
            "the fourth was refused with `{}`, not `{} {said_so}`",
            manager::refused(&refused),
            qvsmsg::MEMORY_NOT_AVAILABLE.id
        ));
    }
    // Nothing of it is left behind.
    let status = manager::status(&mut guest, ids[3])?;
    let after = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let channel = channel(&mut guest)?;
    let groups = channel.ls("/sys/fs/cgroup/qsvs/dedicated")?;
    let left: Vec<&String> = groups.iter().filter(|g| is_partition_group(g)).collect();
    let records = channel.ls("/run/partd")?;
    let fourth = ids[3].to_string();
    if status.state != State::NotActivated
        || status.current.is_some()
        || status.pid != 0
        || after != with_three
        || left.len() != 3
        || records
            .iter()
            .any(|r| r.starts_with(&format!("{fourth}.rec")))
    {
        return Err(format!(
            "after the refusal: the fourth is {} holding {:?}, the groups {left:?}, the \
             records {records:?}",
            status.state.word(),
            status.current
        ));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "three partitions Running together (VMMs {pids:?}), holding 3.00 of {} processing \
         units and {} of {} MB; a fourth refused with `{}`, Not activated, holding nothing, \
         with no control group and no record of it, the accounts as they were",
        Units(u32::try_from(before.processors.configurable).unwrap_or(0)),
        3 * SPIN_MIB,
        before.memory.configurable,
        refused[0]
    ))
}

/// Desired not available but minimum is: the partition activates with
/// what there is and reports it as current.
pub fn minimum(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 2)?, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let host = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let cores = u32::try_from(host.processors.configurable / 100).unwrap_or(0);
    // The first takes all but 256 MiB and all but two cores.
    let mut first = spin(0, cores - 2);
    let most = host.memory.configurable - 2 * SPIN_MIB;
    first.memory_mib = (SPIN_MIB, most);
    run_one(&mut guest, ids[0], &first, deadline)?;

    // The second wants more than is left of both.
    let mut wants = spin(1, 1).profile();
    wants.processors.desired = cores;
    wants.processors.maximum = cores;
    wants.memory.desired_mib = most;
    wants.memory.maximum_mib = most;
    said(partd(&mut guest)?.put_profile(ids[1], wants.clone()))?;
    let got = said(partd(&mut guest)?.activate(ids[1], wants.name.clone(), None, None))?;
    // The second one's line: the first's is still unread.
    let line = format!(
        "{}: {}, ",
        crate::suite::base::posted(&refcode::PARTITION_MACHINE_BUILT, ""),
        name(1)
    );
    guest.expect(&line, deadline)?;
    let built = guest.rest_of_line(deadline)?;
    let status = manager::until(&mut guest, ids[1], State::Running, deadline)?;
    let Processors::Dedicated { cores: given } = &got.processors else {
        return Err(format!("the second was given {:?}", got.processors));
    };
    if given.len() != 2
        || got.memory_mib != 2 * SPIN_MIB
        || status.current.as_ref() != Some(&got)
        || !built.contains("2 processor(s)")
        || !built.contains(&format!("{} MiB", 2 * SPIN_MIB))
    {
        return Err(format!(
            "wanted {cores} processors and {most} MB with 2 and {} MB left: given {got:?}, \
             current {:?}, the VMM built `{built}`",
            2 * SPIN_MIB,
            status.current
        ));
    }
    let after = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    if after.memory.available != 0 || after.processors.available != 0 {
        return Err(format!("both run and {after:?} is left"));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "desired {cores} processors and {most} MB, minimum 1 and {SPIN_MIB} MB, with 2 cores \
         and {} MB left: activated with those, reported as current, and the VMM built `{built}`",
        2 * SPIN_MIB
    ))
}

/// Dedicated: the partition's virtual CPU threads are on its cores
/// and nothing else is, while every partition is busy.
#[allow(clippy::too_many_lines)] // one suite, told in order
pub fn dedicated(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 3)?, 3)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let host = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let mut held: Vec<Vec<u32>> = Vec::new();
    for (i, processors) in [(0usize, 2u32), (1, 1)] {
        let got = run_one(&mut guest, ids[i], &spin(i, processors), deadline)?;
        let Processors::Dedicated { cores } = got.processors else {
            return Err(format!(
                "partition {} was given {:?}",
                name(i),
                got.processors
            ));
        };
        held.push(cores);
    }
    // A shared partition beside them, busy too.
    let shared = spin(2, 1).shared(100, Sharing::Uncapped);
    run_one(&mut guest, ids[2], &shared, deadline)?;
    // The processors of each partition's cores, as its control group
    // has them: a core's worth of threads for each core.
    let mut cpus_of: Vec<Vec<u32>> = Vec::new();
    for (i, cores) in held.iter().enumerate() {
        let file = format!("/sys/fs/cgroup/qsvs/dedicated/p{}/cpuset.cpus", ids[i]);
        let list = channel(&mut guest)?.text(&file)?;
        let cpus = resource::topology::parse_cpu_list(&list).unwrap_or_default();
        if cpus.len() != cores.len() * host.threads_per_core as usize {
            return Err(format!(
                "{} holds the cores {cores:?} and its group the processors `{}`",
                name(i),
                list.trim()
            ));
        }
        cpus_of.push(cpus);
    }
    let dedicated: Vec<u32> = cpus_of.iter().flatten().copied().collect();
    let mut summary = Vec::new();
    for (i, cores) in held.iter().enumerate() {
        let cpus = cpus_of[i].clone();
        let pid = pid_of(&mut guest, ids[i])?;
        // Sampled while the guest spins.
        for _ in 0..5 {
            for thread in threads(&mut guest, pid)? {
                let is_vcpu = thread.name.starts_with("vcpu");
                let at_home = thread.allowed.iter().all(|cpu| cpus.contains(cpu))
                    && cpus.contains(&thread.processor);
                if !at_home || (is_vcpu && thread.allowed.len() != 1) {
                    return Err(format!(
                        "{} has the processors {cpus:?}; its thread {} is allowed {:?} and \
                         last ran on {}",
                        name(i),
                        thread.name,
                        thread.allowed,
                        thread.processor
                    ));
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let vcpus: Vec<String> = threads(&mut guest, pid)?
            .iter()
            .filter(|t| t.name.starts_with("vcpu"))
            .map(|t| format!("{} on {}", t.name, t.allowed[0]))
            .collect();
        // The spinner's one busy processor has been busy.
        let busy = threads(&mut guest, pid)?
            .iter()
            .filter(|t| t.name.starts_with("vcpu"))
            .map(|t| t.ticks)
            .max()
            .unwrap_or(0);
        if busy == 0 {
            return Err(format!("{}'s processors have used no time", name(i)));
        }
        summary.push(format!("{} cores {cores:?}: {}", name(i), vcpus.join(", ")));
    }
    // Nothing else is on those processors. No process of the host but
    // a kernel thread is allowed there:
    let own: Vec<String> = ids[..2]
        .iter()
        .map(|id| format!("/qsvs/dedicated/p{id}"))
        .collect();
    let mut kernel_threads: Vec<u32> = Vec::new();
    let mut others = 0;
    for line in ask(&mut guest, command::TASKS)? {
        let Some(words) = line_words(&line) else {
            continue;
        };
        let (kthread, group) = (words.get(1) == Some(&"1"), words.get(4).copied());
        if kthread {
            kernel_threads.extend(words.first().and_then(|p| p.parse::<u32>().ok()));
            continue;
        }
        if group.is_some_and(|g| own.iter().any(|o| o == g)) {
            continue;
        }
        others += 1;
        let allowed = words
            .get(3)
            .and_then(|list| resource::topology::parse_cpu_list(list))
            .unwrap_or_default();
        if allowed.iter().any(|cpu| dedicated.contains(cpu)) {
            return Err(format!(
                "the processors {dedicated:?} are dedicated, and another process is allowed \
                 there: `{line}`"
            ));
        }
    }
    // and what the scheduler says each of them runs, looked at while
    // every partition is busy, is its partition's thread, a kernel
    // thread, or nothing.
    let mut tids: Vec<Vec<u32>> = Vec::new();
    for id in &ids[..2] {
        let pid = pid_of(&mut guest, *id)?;
        tids.push(threads(&mut guest, pid)?.iter().map(|t| t.tid).collect());
    }
    let (mut looks, mut busy_looks) = (0u32, 0u32);
    for _ in 0..40 {
        for (cpu, pid) in running_now(&mut guest)? {
            let Some(owner) = cpus_of.iter().position(|cpus| cpus.contains(&cpu)) else {
                continue;
            };
            looks += 1;
            if tids[owner].contains(&pid) {
                busy_looks += 1;
            } else if pid != 0 && !kernel_threads.contains(&pid) {
                return Err(format!(
                    "processor {cpu} is {}'s alone and was running process {pid}",
                    name(owner)
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if busy_looks == 0 {
        return Err("no dedicated processor was seen running its partition".into());
    }
    let pool = channel(&mut guest)?.text("/sys/fs/cgroup/qsvs/pools/DEFAULT/cpuset.cpus")?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "{}; every thread of each VMM allowed its partition's processors only and seen on \
         them in five looks; none of the host's {} other processes (kernel threads apart) is \
         allowed {dedicated:?}; in {looks} looks at what those processors run, {busy_looks} \
         found the partition's own thread and the rest nothing or a kernel thread; the \
         default pool is left `{}`",
        summary.join("; "),
        others,
        pool.trim()
    ))
}

/// Shared, capped at 0.50 units with one virtual processor: under
/// full load it measures within the tolerance of half a core.
pub fn capped(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 1)?, 1)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let host = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let spec = spin(0, 1).shared(50, Sharing::Capped);
    let got = run_one(&mut guest, ids[0], &spec, deadline)?;
    let group = resource::cgroup::partition_group(ids[0], &got.processors);
    let max = channel(&mut guest)?.text(&format!("/sys/fs/cgroup/{group}/cpu.max"))?;
    let (threads, _) = measure(&mut guest, std::slice::from_ref(&group))?[0];
    let cores = threads / f64::from(host.threads_per_core);
    if (cores - 0.5).abs() > 0.5 * TOLERANCE {
        return Err(format!(
            "capped at 0.50 units, the partition used {cores:.3} of a core over {} s \
             (cpu.max `{}`)",
            MEASURE.as_secs(),
            max.trim()
        ));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "one virtual processor capped at 0.50 units, a guest that only runs: {cores:.3} of a \
         core over {} s, within {:.0}% of half (cpu.max `{}`, {} thread(s) a core). Run under \
         nested virtualization: the tolerance is the handoff's 10%, not the 5% of a machine \
         that is no virtual machine",
        MEASURE.as_secs(),
        100.0 * TOLERANCE,
        max.trim(),
        host.threads_per_core
    ))
}

/// Shared, uncapped: two partitions of equal weight and different
/// entitlements, contending for one core, each get at least their
/// entitlement, and the spare time splits by weight.
pub fn uncapped(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 2)?, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let host = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    // A pool of one core: the last the host has.
    let core = host.cores - 1;
    said(partd(&mut guest)?.create_pool(Pool {
        name: "TIGHT".into(),
        cores: vec![core],
        maximum_units: None,
    }))?;
    let mut groups = Vec::new();
    let entitled = [20u32, 60];
    for (i, units) in entitled.into_iter().enumerate() {
        let mut spec = spin(i, 1).shared(units, Sharing::Uncapped);
        spec.weight = Some(128);
        let mut profile = spec.profile();
        profile.processors.pool = Some("TIGHT".into());
        said(partd(&mut guest)?.put_profile(ids[i], profile.clone()))?;
        let got = said(partd(&mut guest)?.activate(ids[i], profile.name, None, None))?;
        manager::until(&mut guest, ids[i], State::Running, deadline)?;
        groups.push(resource::cgroup::partition_group(ids[i], &got.processors));
    }
    let weights: Vec<String> = {
        let channel = channel(&mut guest)?;
        groups
            .iter()
            .map(|g| {
                channel
                    .text(&format!("/sys/fs/cgroup/{g}/cpu.weight"))
                    .map(|w| w.trim().to_string())
            })
            .collect::<Result<_, _>>()?
    };
    let measured = measure(&mut guest, &groups)?;
    let per_core = f64::from(host.threads_per_core);
    let shares: Vec<f64> = measured.iter().map(|(t, _)| t / per_core).collect();
    let spare = 1.0 - 0.8;
    // Of the spare 0.20, a quarter and three quarters: the weights are
    // equal, so the kernel's shares are the entitlements'.
    let expected = [0.20 + spare * 0.25, 0.60 + spare * 0.75];
    for i in 0..2 {
        let least = f64::from(entitled[i]) / 100.0;
        if shares[i] < least * (1.0 - TOLERANCE)
            || (shares[i] - expected[i]).abs() > expected[i] * 2.0 * TOLERANCE
        {
            return Err(format!(
                "entitled to 0.20 and 0.60 of one core, uncapped at equal weight: they used \
                 {:.3} and {:.3} (cpu.weight {weights:?})",
                shares[0], shares[1]
            ));
        }
    }
    if shares[0] + shares[1] < 0.9 {
        return Err(format!(
            "two busy partitions used {:.3} of the pool's one core",
            shares[0] + shares[1]
        ));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "a pool of one core, two uncapped partitions of weight 128 entitled to 0.20 and 0.60, \
         both only running: {:.3} and {:.3} of the core over {} s (cpu.weight {} and {}): \
         each at least its entitlement, the spare 0.20 split one to three",
        shares[0],
        shares[1],
        MEASURE.as_secs(),
        weights[0],
        weights[1]
    ))
}

/// What each processor runs now, by the scheduler's own account.
fn running_now(guest: &mut Session) -> Result<Vec<(u32, u32)>, String> {
    let lines = ask(guest, command::CURR)?;
    Ok(lines
        .iter()
        .filter_map(|line| {
            let words = line_words(line)?;
            Some((words.first()?.parse().ok()?, words.get(1)?.parse().ok()?))
        })
        .collect())
}

/// Core scheduling: two shared partitions never run on the two
/// threads of one core at the same time, sampled over a loaded run.
#[allow(clippy::too_many_lines)] // one suite, told in order
pub fn core_scheduling(run: &Run<'_>) -> Result<String, String> {
    // One socket of three cores of two threads.
    let mut l = spinner_launch(run, 2)?;
    l.topology = Some((1, 3, 2));
    let (mut guest, ids) = spinners(run, &l, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let host = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    if host.threads_per_core != 2 || host.cores != 3 {
        return Err(format!(
            "the host was to have three cores of two threads; it has {} of {}",
            host.cores, host.threads_per_core
        ));
    }
    // A pool of one core: both partitions have nowhere else to go.
    said(partd(&mut guest)?.create_pool(Pool {
        name: "ONE".into(),
        cores: vec![2],
        maximum_units: None,
    }))?;
    let mut groups = Vec::new();
    let mut vcpus: Vec<Vec<u32>> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let mut profile = spin(i, 1).shared(50, Sharing::Uncapped).profile();
        profile.processors.pool = Some("ONE".into());
        said(partd(&mut guest)?.put_profile(*id, profile.clone()))?;
        let got = said(partd(&mut guest)?.activate(*id, profile.name, None, None))?;
        let built = expect_code(&mut guest, &refcode::PARTITION_MACHINE_BUILT, deadline)?;
        if !built.contains("1 processor(s) of 2 thread(s)") {
            return Err(format!(
                "a virtual processor of a host with SMT is two threads: `{built}`"
            ));
        }
        manager::until(&mut guest, *id, State::Running, deadline)?;
        groups.push(resource::cgroup::partition_group(*id, &got.processors));
        let pid = pid_of(&mut guest, *id)?;
        vcpus.push(threads(&mut guest, pid)?.iter().map(|t| t.tid).collect());
    }
    let pool_cpus = channel(&mut guest)?.text("/sys/fs/cgroup/qsvs/pools/ONE/cpuset.cpus")?;
    let siblings = resource::topology::parse_cpu_list(&pool_cpus).unwrap_or_default();
    if siblings.len() != 2 {
        return Err(format!(
            "the pool of one core has the processors `{pool_cpus}`"
        ));
    }
    if guest.said("qsvmm: core scheduling: the host has none to do") {
        return Err("qsvmm says the host has no core scheduling".into());
    }
    // Sampled while both spin: what the core's two threads run.
    let whose = |pid: u32| vcpus.iter().position(|tids| tids.contains(&pid));
    let (mut samples, mut each, mut together) = (0u32, [0u32; 2], 0u32);
    let began = Instant::now();
    let before: Vec<(u64, f64, u64)> = groups
        .iter()
        .map(|g| usage(&mut guest, g))
        .collect::<Result<_, _>>()?;
    while began.elapsed() < MEASURE {
        let now = running_now(&mut guest)?;
        let on: Vec<Option<usize>> = siblings
            .iter()
            .map(|cpu| {
                now.iter()
                    .find(|(c, _)| c == cpu)
                    .and_then(|(_, pid)| whose(*pid))
            })
            .collect();
        samples += 1;
        for partition in on.iter().flatten() {
            each[*partition] += 1;
        }
        if let (Some(a), Some(b)) = (on[0], on[1])
            && a != b
        {
            // Looked at again at once: the two processors are not
            // read at one instant.
            let again = running_now(&mut guest)?;
            let still = siblings.iter().all(|cpu| {
                again.iter().find(|(c, _)| c == cpu).map(|(_, p)| *p)
                    == now.iter().find(|(c, _)| c == cpu).map(|(_, p)| *p)
            });
            together += u32::from(still);
        }
    }
    let after: Vec<(u64, f64, u64)> = groups
        .iter()
        .map(|g| usage(&mut guest, g))
        .collect::<Result<_, _>>()?;
    #[allow(clippy::cast_precision_loss)]
    let busy: f64 = (0..2)
        .map(|i| (after[i].0 - before[i].0) as f64 / 1e6 / (after[i].1 - before[i].1))
        .sum();
    let forced: u64 = (0..2).map(|i| after[i].2 - before[i].2).sum();
    if together > 0 || each[0] == 0 || each[1] == 0 {
        return Err(format!(
            "in {samples} looks at the core's two threads, both partitions ran at once \
             {together} time(s); each was seen running {each:?} times"
        ));
    }
    // One of them at a time: a thread's worth of time between them,
    // where two would be used without core scheduling.
    if busy > 1.2 {
        return Err(format!(
            "two partitions on one core of two threads used {busy:.2} threads' time together"
        ));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "a pool of one core (processors {}), two shared partitions each with one busy virtual \
         CPU: in {samples} looks at what the core's two threads run, {} and {} with a \
         partition on a thread, never the two partitions at once; together they used {busy:.2} \
         threads' time over {} s, and were kept idle beside each other for {} ms",
        pool_cpus.trim(),
        each[0],
        each[1],
        MEASURE.as_secs(),
        forced / 1000
    ))
}

/// The value of `key` in `/proc/meminfo`, in its unit.
fn meminfo(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|rest| rest.split_ascii_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// The kibibytes of a process's mappings that are 2 MB pages.
fn huge_kib(smaps: &str) -> u64 {
    let mut total = 0;
    let mut size = 0;
    for line in smaps.lines() {
        if let Some(rest) = line.strip_prefix("Size:") {
            size = rest
                .split_ascii_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
        } else if line.starts_with("KernelPageSize:") && line.contains(" 2048 kB") {
            total += size;
        }
    }
    total
}

/// Huge pages: guest memory is on 2 MB pages; a host started with too
/// few posts the code and reports the lower configurable memory.
pub fn huge_pages(run: &Run<'_>) -> Result<String, String> {
    let l = spinner_launch(run, 1)?;
    let (mut guest, ids) = spinners(run, &l, 1)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let reserved = guest.said(&posted(&refcode::MEMORY_RESERVED, ""));
    let host = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let before = channel(&mut guest)?.text("/proc/meminfo")?;
    let pages = meminfo(&before, "HugePages_Total");
    if !reserved
        || pages * 2 != host.memory.configurable
        || meminfo(&before, "Hugepagesize") != 2048
    {
        return Err(format!(
            "the host says {} MB are configurable and has {pages} huge pages of {} kB \
             (C2001072 posted: {reserved})",
            host.memory.configurable,
            meminfo(&before, "Hugepagesize")
        ));
    }
    let mut spec = spin(0, 1);
    spec.memory_mib = (256, 256);
    run_one(&mut guest, ids[0], &spec, deadline)?;
    let pid = pid_of(&mut guest, ids[0])?;
    let channel = channel(&mut guest)?;
    let smaps = channel.text(&format!("/proc/{pid}/smaps"))?;
    let after = channel.text("/proc/meminfo")?;
    let mapped = huge_kib(&smaps);
    let taken = meminfo(&before, "HugePages_Free") - meminfo(&after, "HugePages_Free")
        + meminfo(&after, "HugePages_Rsvd");
    if mapped != 256 * 1024 || taken != 128 {
        return Err(format!(
            "a partition of 256 MB: {mapped} kB of its VMM's mappings are 2 MB pages, and \
             {taken} huge pages are taken or reserved"
        ));
    }

    // A reserve too small to stay up on is refused.
    let mut settings = partd(&mut guest)?.settings().map_err(|e| e.to_string())?;
    settings.reserved_memory_mib = Some(96);
    let refused = match partd(&mut guest)?.change_settings(settings) {
        Ok(Err(refused)) if refused.iter().any(|m| m.id == qvsmsg::SETTING_INVALID.id) => {
            manager::refused(&refused)
        }
        other => return Err(format!("a reserve of 96 MB: {other:?}")),
    };

    // Too few: at the next IPL the host's reserve is held by PID 1
    // while it asks for the pages (a hook of the test image), so the
    // kernel cannot give all of them.
    let wanted = host.memory.configurable;
    super::power_off(&mut guest, 3 * run.timeout)?;
    drop(guest);
    let mut l = l;
    l.oem.push("qsvs.hook=hold-memory".into());
    let mut guest = super::host(run, &l)?;
    let short = expect_posted(&mut guest, &refcode::MEMORY_SHORT)?;
    let now = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let given = meminfo(
        &super::super::base::channel(&mut guest)?.text("/proc/meminfo")?,
        "HugePages_Total",
    );
    if now.memory.configurable >= wanted || now.memory.configurable != given * 2 / 16 * 16 {
        return Err(format!(
            "with the reserve held, of {wanted} MB wanted: {} MB configurable, {given} huge pages \
             (`{short}`)",
            now.memory.configurable
        ));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "{pages} huge pages reserved at the IPL for {} MB configurable; a partition of 256 MB \
         has {mapped} kB of 2 MB pages mapped and takes 128 pages. A reserve of 96 MB is \
         refused (`{refused}`). With its reserve held while it asks, the host posts `A2001072 \
         {short}` and reports {} MB configurable of {} installed, where {wanted} were asked for",
        host.memory.configurable, now.memory.configurable, now.memory.installed
    ))
}

/// What a code the host posted at its IPL added to its text.
fn expect_posted(guest: &mut Session, code: &refcode::RefCode) -> Result<String, String> {
    guest.drain();
    let mark = posted(code, "");
    guest
        .log
        .rfind(&mark)
        .and_then(|at| guest.log[at + mark.len()..].lines().next())
        .map(|rest| rest.trim_start_matches(':').trim().to_string())
        .ok_or_else(|| format!("the host did not post {}", code.code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kernel_s_accounts_are_read() {
        let meminfo_text = "MemTotal:  5000000 kB\nHugePages_Total:    1440\n\
                            HugePages_Free:     1312\nHugePages_Rsvd:       64\n\
                            Hugepagesize:       2048 kB\n";
        assert_eq!(meminfo(meminfo_text, "HugePages_Total"), 1440);
        assert_eq!(meminfo(meminfo_text, "Hugepagesize"), 2048);
        assert_eq!(meminfo(meminfo_text, "Nothing"), 0);
        let smaps = "7f00-7f10 rw-s 00000000 00:0f 1 /anon_hugepage (deleted)\n\
                     Size:             262144 kB\nKernelPageSize:     2048 kB\n\
                     MMUPageSize:        2048 kB\n\
                     5500-5510 r-xp 00000000 00:02 9 /qsvs/bin/qsvmm\n\
                     Size:               1024 kB\nKernelPageSize:        4 kB\n";
        assert_eq!(huge_kib(smaps), 262_144);
        assert_eq!(device(0), "/dev/vdb");
        assert_eq!(name(2), "P3");
        assert_eq!(
            line_words("line 12 R 3 40 3 vcpu0"),
            Some(vec!["12", "R", "3", "40", "3", "vcpu0"])
        );
    }

    #[test]
    fn a_spinner_s_profile_is_valid_dedicated_or_shared() {
        let limits = model::Limits {
            memory_block_mib: 16,
        };
        for spec in [spin(0, 2), spin(1, 1).shared(50, Sharing::Capped)] {
            assert_eq!(model::validate::profile(&spec.profile(), limits), []);
        }
    }
}
