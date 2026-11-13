// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What happens around partitions: they start with the host, one
//! whose VMM is killed leaves the others alone, the partition manager
//! is started again under them, the host's shutdown ends them first,
//! each VMM is its own user, and Service Tools lists them. The guests
//! only run (`ipl::load::spinner`).

use std::time::{Duration, Instant};

use model::profile::Sharing;
use partd::{Calls as _, State};
use qsvshook::proto::command;

use super::manager::{self, partd, said};
use super::resources::{device, name, run_one, spin, spinner_launch, spinners, threads};
use super::{ask, host, host_unaffected, pid_of, power_off, signal_to};
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel, posted};

/// Where in the serial log `text` first is, after `from`.
fn position(guest: &mut Session, text: &str, from: usize) -> Result<usize, String> {
    guest.drain();
    guest.log[from..]
        .find(text)
        .map(|at| from + at)
        .ok_or_else(|| format!("`{text}` is not on the serial port"))
}

/// Auto start: flagged partitions return in order after a host
/// restart; one with a missing backing store fails alone.
pub fn auto_start(run: &Run<'_>) -> Result<String, String> {
    let l = spinner_launch(run, 2)?;
    let (mut guest, ids) = spinners(run, &l, 3)?;
    // P1 last, P2 before it after two seconds, P3 first, and P3's
    // disk is not there.
    for (i, order, delay) in [(0usize, 20u32, 0u32), (1, 10, 2), (2, 5, 0)] {
        let mut spec = spin(i, 1);
        if i == 2 {
            spec.backing = vec!["/dev/vdz".into()];
        }
        manager::put(&mut guest, ids[i], &spec)?;
        manager::auto_start(&mut guest, ids[i], order, delay)?;
    }
    if partd(&mut guest)?
        .list()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|p| p.state != State::NotActivated)
    {
        return Err("flagging a partition to start with the host activated it".into());
    }
    power_off(&mut guest, 3 * run.timeout)?;
    drop(guest);

    // The host's next IPL.
    let mut guest = host(run, &l)?;
    let deadline = Instant::now() + 3 * run.timeout;
    guest.expect(&posted(&refcode::AUTO_START_STARTED, ""), deadline)?;
    let began = Instant::now();
    guest.expect(&posted(&refcode::AUTO_START_DONE, ""), deadline)?;
    let done = guest.rest_of_line(deadline)?;
    for id in &ids[..2] {
        manager::until(&mut guest, *id, State::Running, deadline)?;
    }
    let failed = manager::status(&mut guest, ids[2])?;
    let why = failed.message.map(|m| m.to_string()).unwrap_or_default();
    if failed.state != State::NotActivated
        || !why.starts_with(qvsmsg::BACKING_MISSING.id)
        || !done.contains("2 of 3 activated")
    {
        return Err(format!(
            "the partition whose disk is not there is {} (`{why}`); auto start ended `{done}`",
            failed.state.word()
        ));
    }
    // In order: P3 fails first, then P2, then P1.
    let third = position(
        &mut guest,
        &posted(&refcode::AUTO_START_FAILED, &name(2)),
        0,
    )?;
    let activating = |n: usize| posted(&refcode::PARTITION_ACTIVATING, &name(n));
    let second = position(&mut guest, &activating(1), third)?;
    let first = position(&mut guest, &activating(0), second)?;
    if began.elapsed() < Duration::from_secs(2) {
        return Err("the delay before a partition was not waited".into());
    }
    // Once an IPL: the partition manager started again does not do it over.
    let before = guest
        .log
        .matches(&posted(&refcode::AUTO_START_STARTED, ""))
        .count();
    ask(&mut guest, &format!("{} partd", command::KILL))?;
    let give_up = Instant::now() + run.timeout;
    while partd(&mut guest)?.list().is_err() {
        if Instant::now() >= give_up {
            return Err("the partition manager did not come back".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_secs(3));
    guest.drain();
    if guest
        .log
        .matches(&posted(&refcode::AUTO_START_STARTED, ""))
        .count()
        != before
    {
        return Err("auto start ran again when the partition manager was started again".into());
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "three partitions flagged, at places 20, 10 (after 2 s) and 5, the last with a \
         backing store that is not there: after the host's restart `C2006020`, then \
         `A2006020` for {} (`{why}`), then {} and {} activated in that order ({} bytes apart \
         on the serial port), `C2006030 {}`; both Running; not done over when the partition \
         manager was started again",
        name(2),
        name(1),
        name(0),
        first - second,
        done.trim_start_matches(':').trim()
    ))
}

/// A killed `qsvmm`: the partition goes to Error with its reference
/// code, the others and the host are untouched, and it activates
/// again afterwards.
pub fn killed(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 3)?, 3)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let mut pids = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        run_one(&mut guest, *id, &spin(i, 1), deadline)?;
        pids.push(pid_of(&mut guest, *id)?);
    }
    let before = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
    signal_to(&mut guest, ids[1], "KILL")?;
    let status = manager::ended(&mut guest, ids[1], deadline)?;
    let message = status.message.map(|m| m.to_string()).unwrap_or_default();
    if status.state != State::Error
        || status.code != refcode::PARTITION_KILLED.code
        || status.last_end != "signal 9"
        || status.current.is_some()
        || !message.starts_with(qvsmsg::VMM_FAILED.id)
    {
        return Err(format!(
            "the killed partition is {} with `{} {}`, ended `{}`: {message}",
            status.state.word(),
            status.code,
            status.text,
            status.last_end
        ));
    }
    guest.seen(
        &posted(
            &refcode::PARTITION_KILLED,
            &format!("{}, signal 9", name(1)),
        ),
        deadline,
    )?;
    // The others are as they were, with the VMMs they had.
    std::thread::sleep(Duration::from_secs(2));
    let after = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
    for i in [0usize, 2] {
        if after[i] != before[i] || pid_of(&mut guest, ids[i])? != pids[i] {
            return Err(format!(
                "{} was {:?} and is {:?} after another's VMM was killed",
                name(i),
                before[i],
                after[i]
            ));
        }
    }
    host_unaffected(&mut guest)?;
    // It was not started again by itself, and is when asked.
    if manager::status(&mut guest, ids[1])?.state != State::Error {
        return Err("a partition in error was started again by itself".into());
    }
    run_one(&mut guest, ids[1], &spin(1, 1), deadline)?;
    let again = pid_of(&mut guest, ids[1])?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "three partitions Running; the VMM of {} (process {}) killed: Error, `A2005150 {}`, \
         holding nothing, `{message}`; {} and {} Running as before with the VMMs they had, \
         the host's base ready; activated again, process {again}",
        name(1),
        pids[1],
        status.text,
        name(0),
        name(2)
    ))
}

/// The partition manager killed and started again with partitions
/// running: they keep running and are watched again; then the host
/// powers off cleanly and every partition is ended first.
pub fn manager_restart(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 2)?, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    // The second is activated first: the order of ending is not the
    // order of the IDs.
    run_one(&mut guest, ids[1], &spin(1, 1), deadline)?;
    let shared = spin(0, 1).shared(50, Sharing::Capped);
    run_one(&mut guest, ids[0], &shared, deadline)?;
    let before = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
    let resources = partd(&mut guest)?.resources().map_err(|e| e.to_string())?;
    let manager_pid = channel(&mut guest)?
        .status()?
        .service("partd")
        .map_or(0, |s| s.pid);

    ask(&mut guest, &format!("{} partd", command::KILL))?;
    guest.expect(
        &posted(&refcode::PARTD_STARTED, "2 defined, 2 running"),
        deadline,
    )?;
    let give_up = Instant::now() + run.timeout;
    let after = loop {
        match partd(&mut guest)?.list() {
            Ok(all) => break all,
            Err(e) if Instant::now() >= give_up => {
                return Err(format!("the partition manager did not come back: {e}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    };
    let new_pid = channel(&mut guest)?
        .status()?
        .service("partd")
        .map_or(0, |s| s.pid);
    for i in 0..2 {
        let (was, is) = (&before[i], &after[i]);
        if is.state != State::Running
            || is.pid != was.pid
            || is.current != was.current
            || is.code != refcode::PARTITION_ADOPTED.code
        {
            return Err(format!("{} was {was:?} and is {is:?}", name(i)));
        }
        // The VMM itself went on running.
        let busy = threads(&mut guest, is.pid)?
            .iter()
            .any(|t| t.name.starts_with("vcpu") && t.state == "R");
        if !busy {
            return Err(format!("the VMM of {} is not running its guest", name(i)));
        }
    }
    if new_pid == manager_pid
        || partd(&mut guest)?.resources().map_err(|e| e.to_string())? != resources
    {
        return Err("the partition manager's accounts changed over its restart".into());
    }
    // They are watched again: one that is shut down is seen to end,
    // with the exit code its VMM left for whoever could not wait.
    said(partd(&mut guest)?.shut_down(ids[0]))?;
    let ended = manager::ended(&mut guest, ids[0], deadline)?;
    if ended.state != State::NotActivated || ended.last_end != "exit 50" {
        return Err(format!(
            "shut down after the restart, {} is {} having ended `{}`",
            name(0),
            ended.state.word(),
            ended.last_end
        ));
    }
    run_one(&mut guest, ids[0], &shared, deadline)?;
    host_unaffected(&mut guest)?;

    // The host powers off: the partitions are ended first, the last
    // activated first.
    channel(&mut guest)?.tell(command::POWER_OFF)?;
    let limit = Instant::now() + 3 * run.timeout;
    let started =
        position(&mut guest, &posted(&refcode::SHUTDOWN_STARTED, ""), 0).or_else(|_| {
            guest.expect(&posted(&refcode::SHUTDOWN_STARTED, ""), limit)?;
            position(&mut guest, &posted(&refcode::SHUTDOWN_STARTED, ""), 0)
        })?;
    guest.expect(&posted(&refcode::PARTITIONS_ENDED, "2 partition(s)"), limit)?;
    guest.expect(&posted(&refcode::POWER_OFF, ""), limit)?;
    guest.wait_exit(limit)?;
    let end_of = |n: usize| posted(&refcode::PARTITION_ENDED, &format!("{}, exit 50", name(n)));
    let first = position(&mut guest, &end_of(0), started)?;
    let second = position(&mut guest, &end_of(1), first)?;
    let all_ended = position(&mut guest, &posted(&refcode::PARTITIONS_ENDED, ""), second)?;
    if guest.said("was killed") || guest.said("not unmounted") {
        return Err("the shutdown under partitions was not clean".into());
    }
    Ok(format!(
        "the partition manager (process {manager_pid}) killed with two partitions Running: \
         started again as process {new_pid}, `C2006000 Partition manager started: 2 defined, \
         2 running`, both `C2006010`, Running with the VMMs and the resources they had, their \
         guests still running; one shut down and activated again under the new manager (exit \
         50 read from its report); at the host's power off `D2001000`, then {} ended, then {} \
         ({} bytes on), then `D2001010 Partitions ended: 2 partition(s)` ({} bytes on), then \
         `D2001090`",
        name(0),
        name(1),
        second - first,
        all_ended - second
    ))
}

/// Isolation: each `qsvmm` runs as its own user; one cannot signal
/// another or read its log.
pub fn isolation(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 2)?, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let mut pids = Vec::new();
    let mut users = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        run_one(&mut guest, *id, &spin(i, 1), deadline)?;
        let pid = pid_of(&mut guest, *id)?;
        let user = model::definition::user_of(*id);
        let status = channel(&mut guest)?.text(&format!("/proc/{pid}/status"))?;
        super::devices::unprivileged(&status, &format!("the VMM of {}", name(i)), user)?;
        pids.push(pid);
        users.push(user);
    }
    if users[0] == users[1] {
        return Err(format!("both VMMs are user {}", users[0]));
    }
    let log = |id: u32| {
        qsvmm::log_dir(std::path::Path::new(release::DATA_DIR), id)
            .join(qsvmm::SERIAL_LOG)
            .to_string_lossy()
            .into_owned()
    };
    let may = |guest: &mut Session, user: u32, verb: &str, what: &str| -> Result<String, String> {
        let line = format!("{} {user} {verb} {what}", command::AS);
        match channel(guest)?.ask(&line) {
            Ok(_) => Ok("allowed".into()),
            Err(why) => Ok(why),
        }
    };
    let mut seen = Vec::new();
    for (me, other) in [(0usize, 1usize), (1, 0)] {
        // Its own: allowed. The check that refuses is then a refusal.
        for (verb, what) in [("kill", pids[me].to_string()), ("read", log(ids[me]))] {
            let own = may(&mut guest, users[me], verb, &what)?;
            if own != "allowed" {
                return Err(format!(
                    "user {} may not {verb} its own {what}: {own}",
                    users[me]
                ));
            }
        }
        let signal = may(&mut guest, users[me], "kill", &pids[other].to_string())?;
        let read = may(&mut guest, users[me], "read", &log(ids[other]))?;
        if signal == "allowed" || read == "allowed" {
            return Err(format!(
                "user {} of {}: signalling the VMM of {} is `{signal}`, reading its log is \
                 `{read}`",
                users[me],
                name(me),
                name(other)
            ));
        }
        seen.push(format!(
            "user {} may signal and read its own, and of {}: `{signal}`, `{read}`",
            users[me],
            name(other)
        ));
    }
    // And the VMMs are still there.
    for i in 0..2 {
        if pid_of(&mut guest, ids[i])? != pids[i] {
            return Err(format!("the VMM of {} is gone", name(i)));
        }
    }
    host_unaffected(&mut guest)?;
    Ok(seen.join("; "))
}

/// Service Tools option 1 lists the partitions.
pub fn tools(run: &Run<'_>) -> Result<String, String> {
    use crate::qemu::term::key;
    use crate::suite::install::{
        Kind, PASSWORD, USER, boot, golden, installed, machine, option, sign_on, titled,
    };
    let disks = installed(run, Kind::Single)?;
    let spinner = spinner_launch(run, 2)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    launch.keys = true;
    launch.memory_mb = super::HOST_MEMORY_MB;
    launch.cpus = super::HOST_CPUS;
    launch.prealloc = true;
    for (i, drive) in spinner.extra.iter().enumerate() {
        let mut drive = drive.clone();
        drive.place = Some(u8::try_from(4 + i).unwrap_or(4));
        launch.extra.push(drive);
    }
    let mut guest = boot(run, &launch)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let give_up = run.deadline();
    while partd(&mut guest)?.list().is_err() {
        if Instant::now() >= give_up {
            return Err("the partition manager does not answer".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // Three partitions: one dedicated, one shared, one not activated.
    let mut ids = Vec::new();
    for name in ["PROD", "SHARED", "DEV"] {
        ids.push(manager::partition(&mut guest, name)?);
    }
    run_one(&mut guest, ids[0], &spin(0, 1), deadline)?;
    let shared = spin(1, 1).shared(50, Sharing::Capped);
    run_one(&mut guest, ids[1], &shared, deadline)?;
    manager::put(&mut guest, ids[2], &spin(0, 1))?;

    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    let status = option(&mut guest, 1, "Work with System Status", deadline)?;
    golden("tools-status-partitions", &status)?;
    guest.type_keys(key::F11)?;
    let list = titled(&mut guest, "Display Partitions", deadline)?;
    golden("tools-partitions", &list)?;
    // It follows the partitions: one is shut down under it.
    said(partd(&mut guest)?.shut_down(ids[1]))?;
    let followed = guest.screen(
        "the list after a partition was shut down",
        deadline,
        |grid| grid.contains("SHARED      Not activated"),
    )?;
    // Nothing on it acts: every key but its own leaves it as it is.
    guest.type_keys(&format!("9{}", key::ENTER))?;
    std::thread::sleep(Duration::from_secs(2));
    let now: Vec<State> = partd(&mut guest)?
        .list()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|p| p.state)
        .collect();
    if now != [State::Running, State::NotActivated, State::NotActivated] {
        return Err(format!("after keys on the list the partitions are {now:?}"));
    }
    guest.type_keys(key::F12)?;
    titled(&mut guest, "Work with System Status", deadline)?;
    let rows = list.lines().filter(|l| l.contains(" MB")).count();
    Ok(format!(
        "option 1 counts the partitions (`3 defined, 2 running`) and F11 lists them: {rows} \
         rows with what each holds, {} without; the list followed a shut down ({} \
         characters) and no key on it acted on a partition; golden files \
         tools-status-partitions and tools-partitions; the units are {} and {}",
        3 - rows,
        followed.len(),
        device(0),
        device(1)
    ))
}
