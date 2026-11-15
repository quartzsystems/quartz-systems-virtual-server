// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The channel itself: the system's codes and states as they arrive,
//! what survives the partition manager and the host started again, a
//! system that stops answering, one that sends nonsense, and one that
//! has no channel at all.

use std::time::{Duration, Instant};

use partd::{Calls as _, State};
use qsvshook::proto::command;

use super::{
    ask, history, ipl_in_order, ipl_patience, qs400_host, qs400_spec, screen, serial, system_codes,
    up, view,
};
use crate::qemu::image::blank;
use crate::suite::base::{Run, base_ready, channel, posted};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::partition::{
    QSVS_MEMORY_MIB, fresh_host, host, host_unaffected, launch, pid_of, signal_to, unit,
};

/// Wait for the partition manager to answer again, after it was
/// started again.
fn manager_back(
    guest: &mut crate::qemu::session::Session,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        if partd(guest)?.list().is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("the partition manager did not answer again".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Suite 2: an IPL's codes arrive in order, each once, and the history
/// on disk holds them after the partition manager and then the host are
/// started again.
pub fn codes(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    let took = up(&mut guest, id, &qs400_spec(None), ipl_patience(run))?;
    let first = system_codes(&history(&mut guest, id)?);
    ipl_in_order(&first)?;
    // The partition manager killed: PID 1 starts it again.
    ask(&mut guest, &format!("{} partd", command::KILL))?;
    std::thread::sleep(Duration::from_secs(2));
    manager_back(&mut guest, run.deadline())?;
    let after = system_codes(&history(&mut guest, id)?);
    if after[..first.len().min(after.len())] != first[..] {
        return Err(format!("after partd: {after:?}, before: {first:?}"));
    }
    ipl_in_order(&after)?;
    // The host restarted: the partition does not outlive it, its history does.
    said(partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    channel(&mut guest)?.tell(command::RESTART)?;
    base_ready(&mut guest, Instant::now() + 3 * run.timeout)?;
    manager_back(&mut guest, run.deadline())?;
    let kept = history(&mut guest, id)?;
    let codes = system_codes(&kept);
    ipl_in_order(&codes)?;
    let hosts = kept
        .iter()
        .filter(|p| p.source == partd::proto::Source::Host)
        .count();
    Ok(format!(
        "QS/400 IPLed in {took:.0} s: its {} codes from C1001000 to C6001090 in the history in \
         order, each once, beside {hosts} of the host's; the same after partd was killed and \
         started again, and after the host restarted ({} entries kept)",
        codes.len(),
        kept.len()
    ))
}

/// Suite 3: Starting until the system reports its IPL complete, then
/// Running; the list shows the release, fix level and an address.
pub fn states(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    manager::activate(&mut guest, id, &qs400_spec(None)).map_err(|e| manager::refused(&e))?;
    let deadline = ipl_patience(run);
    let mut seen = Vec::new();
    let running = loop {
        let s = manager::status(&mut guest, id)?;
        if seen.last() != Some(&s.state) {
            seen.push(s.state);
        }
        if s.state == State::Running {
            break s;
        }
        if Instant::now() >= deadline {
            return Err(format!("the states were {seen:?}"));
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    if seen != [State::Starting, State::Running] {
        return Err(format!("the states were {seen:?}"));
    }
    // Running only when the IPL was complete.
    if !system_codes(&history(&mut guest, id)?).contains(&"C6001090".to_string()) {
        return Err("Running before the system reported its IPL complete".into());
    }
    if running.service.release.is_empty() || !running.service.up {
        return Err(format!("the list's service: {:?}", running.service));
    }
    // An address of its own: an interface added and started on its
    // service console.
    serial(
        &mut guest,
        id,
        "ADDTCPIFC INTNETADR('10.9.9.9') LIND(*VIRTUALIP) SUBNETMASK('255.255.255.255')",
    )?;
    std::thread::sleep(Duration::from_secs(3));
    serial(&mut guest, id, "STRTCPIFC INTNETADR('10.9.9.9')")?;
    let until = Instant::now() + Duration::from_secs(120);
    let s = loop {
        let s = manager::status(&mut guest, id)?;
        if s.service.addresses.iter().any(|a| a == "10.9.9.9/32") {
            break s;
        }
        if Instant::now() >= until {
            return Err(format!("no address in the list: {:?}", s.service));
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    Ok(format!(
        "Starting until C6001090, then Running; the list shows release {}, fix level `{}` (as \
         the system reports it: no cumulative package on this image), addresses {:?}, the \
         channel up at version {}",
        s.service.release, s.service.fix_level, s.service.addresses, s.service.version
    ))
}

/// Suite 12: the partition manager killed with partitions running keeps
/// their states and gets each current code again; the console's frame
/// is whole again on the next attach.
pub fn manager_restart(run: &Run<'_>) -> Result<String, String> {
    let cheap = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    let other = run.file("cheap.img");
    crate::qemu::image::sparse_copy(cheap, &other)?;
    let mut guest = qs400_host(run, &[other])?;
    let id = manager::partition(&mut guest, "PROD")?;
    up(&mut guest, id, &qs400_spec(None), ipl_patience(run))?;
    let second = manager::partition(&mut guest, "CHEAP")?;
    let mut spec = Spec::new(1, QSVS_MEMORY_MIB, "/dev/vdc");
    spec.side = model::IplSource::A;
    manager::activate(&mut guest, second, &spec).map_err(|e| manager::refused(&e))?;
    manager::until(&mut guest, second, State::Running, run.deadline())?;
    let before: Vec<_> = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
    let shown = screen(&mut guest, id)?.text;
    ask(&mut guest, &format!("{} partd", command::KILL))?;
    std::thread::sleep(Duration::from_secs(2));
    manager_back(&mut guest, run.deadline())?;
    // The channel comes up again by itself, and the code with it.
    let until = Instant::now() + Duration::from_secs(60);
    let after = loop {
        let after = partd(&mut guest)?.list().map_err(|e| e.to_string())?;
        if after
            .iter()
            .find(|p| p.id == id)
            .is_some_and(|p| p.service.up)
        {
            break after;
        }
        if Instant::now() >= until {
            return Err("the channel did not come up again".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    for (b, a) in before.iter().zip(&after) {
        if (b.state, &b.code, b.pid) != (a.state, &a.code, a.pid) {
            return Err(format!(
                "{}: {} {} {} before, {} {} {} after",
                b.name,
                b.state.word(),
                b.code,
                b.pid,
                a.state.word(),
                a.code,
                a.pid
            ));
        }
    }
    let viewed = view(&mut guest, id, "view", 2000, &[])?;
    if viewed.fact("frames") != "1" || viewed.rows.join("\n").trim_end() != shown.trim_end() {
        return Err(format!(
            "after the restart a viewer got {} frames:\n{}",
            viewed.fact("frames"),
            viewed.rows.join("\n")
        ));
    }
    Ok(format!(
        "partd killed with PROD (QS/400) and CHEAP (no channel) running: both kept their \
         states ({}, {}), processes and current codes ({}, {}); PROD's channel up again, and a \
         viewer attached got one whole frame equal to the screen",
        after[0].state.word(),
        after[1].state.word(),
        after[0].code,
        after[1].code
    ))
}

/// Suite 13: a system whose daemon is stopped gets the attention code
/// and the event, and loses it when the daemon returns. QS/400 has no
/// way to stop its service daemon alone from outside: the whole VMM is
/// stopped and let go on, which stops the daemon with it.
pub fn silent(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    up(&mut guest, id, &qs400_spec(None), ipl_patience(run))?;
    signal_to(&mut guest, id, "STOP")?;
    let stopped = Instant::now();
    guest.seen(
        &posted(&refcode::NOT_RESPONDING, ""),
        Instant::now() + Duration::from_secs(120),
    )?;
    let silent_after = stopped.elapsed().as_secs_f64();
    let s = manager::status(&mut guest, id)?;
    if s.service.responding || s.code != refcode::NOT_RESPONDING.code || s.state != State::Running {
        return Err(format!(
            "not responding: {} {} {:?}",
            s.state.word(),
            s.code,
            s.service
        ));
    }
    signal_to(&mut guest, id, "CONT")?;
    guest.seen(
        &posted(&refcode::RESPONDING, ""),
        Instant::now() + Duration::from_secs(120),
    )?;
    let s = manager::status(&mut guest, id)?;
    if !s.service.responding || s.code == refcode::NOT_RESPONDING.code {
        return Err(format!("responding again: {} {:?}", s.code, s.service));
    }
    // The events are in the host's log as the manager says them.
    let log = channel(&mut guest)?
        .text("/data/log/system.ndjson")
        .unwrap_or_default();
    let said_silent = log.contains("service channel up, not responding");
    Ok(format!(
        "the VMM stopped: A2009020 after {silent_after:.0} s, the partition still Running and its \
         current code A2009020{}; let go on: C2009020, responding, current code {} again",
        if said_silent {
            ", the event in the host's log"
        } else {
            ""
        },
        s.code
    ))
}

/// Suite 14: a guest that floods its service port, sends oversized and
/// malformed frames, and opens and closes the port quickly does not
/// disturb the partition manager, other partitions or the host; its
/// channel is closed with the code.
pub fn hostile(run: &Run<'_>) -> Result<String, String> {
    let test = run.disk;
    let release = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    fresh_host(run)?;
    let bad = unit(run, test)?;
    let marker = run.file("marker.img");
    blank(&marker, 1 << 20)?;
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&marker)
            .map_err(|e| e.to_string())?;
        f.write_all(qsvshook::partition::hostile::MARKER)
            .map_err(|e| e.to_string())?;
    }
    let good = run.file("good.img");
    crate::qemu::image::sparse_copy(release, &good)?;
    let mut guest = host(run, &launch(run, &[bad, marker, good])?)?;
    let hostile = manager::partition(&mut guest, "HOSTILE")?;
    let calm = manager::partition(&mut guest, "CALM")?;
    let mut spec = Spec::new(1, QSVS_MEMORY_MIB, "/dev/vdb");
    spec.side = model::IplSource::A;
    spec.backing.push("/dev/vdc".into());
    spec.name = "HOSTILE".into();
    let mut calm_spec = Spec::new(1, QSVS_MEMORY_MIB, "/dev/vdd");
    calm_spec.side = model::IplSource::A;
    manager::activate(&mut guest, calm, &calm_spec).map_err(|e| manager::refused(&e))?;
    manager::until(&mut guest, calm, State::Running, run.deadline())?;
    let calm_pid = pid_of(&mut guest, calm)?;
    manager::activate(&mut guest, hostile, &spec).map_err(|e| manager::refused(&e))?;
    let deadline = Instant::now() + 4 * run.timeout;
    guest.seen(&posted(&refcode::CHANNEL_CLOSED, ""), deadline)?;
    // The manager answers throughout, quickly.
    let mut slowest = Duration::ZERO;
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        let asked = Instant::now();
        partd(&mut guest)?.list().map_err(|e| e.to_string())?;
        slowest = slowest.max(asked.elapsed());
        std::thread::sleep(Duration::from_millis(500));
    }
    guest.seen(
        &qsvshook::partition::relayed("HOSTILE", "hostile: done"),
        Instant::now() + 3 * run.timeout,
    )?;
    let h = manager::status(&mut guest, hostile)?;
    let c = manager::status(&mut guest, calm)?;
    if c.state != State::Running || pid_of(&mut guest, calm)? != calm_pid {
        return Err(format!("the calm partition is {}", c.state.word()));
    }
    if !h.state.active() {
        return Err(format!("the hostile partition is {}", h.state.word()));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "a test image as a partition flooded its service port with 2 MiB of junk, frames that do \
         not read, frames too long, 5000 frames of a few kilobytes, and opened and closed it 1000 \
         times: its channel was closed with A2009010 and it ran on ({}); the manager answered a \
         list in {} ms at most; the other partition ran on in the same VMM; every service of the \
         host ready",
        h.state.word(),
        slowest.as_millis()
    ))
}

/// Suite 15: a system with no service channel is Running once its
/// kernel is entered and the wait is over, is marked as having none,
/// and refuses delayed and operating-system shut down by name.
pub fn none(run: &Run<'_>) -> Result<String, String> {
    let release = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    fresh_host(run)?;
    let disk = unit(run, release)?;
    let mut guest = host(run, &launch(run, &[disk])?)?;
    let id = manager::partition(&mut guest, "OLD")?;
    let mut spec = Spec::new(1, QSVS_MEMORY_MIB, "/dev/vdb");
    spec.side = model::IplSource::A;
    let entered = Instant::now();
    manager::activate(&mut guest, id, &spec).map_err(|e| manager::refused(&e))?;
    let s = manager::until(&mut guest, id, State::Running, run.deadline())?;
    let after = entered.elapsed().as_secs_f64();
    if !s.service.none || s.service.up {
        return Err(format!("the list's service: {:?}", s.service));
    }
    let mut refused = Vec::new();
    for kind in [
        partd::proto::Shutdown::Delayed,
        partd::proto::Shutdown::System,
    ] {
        match partd(&mut guest)?.shut_down_as(id, kind, 30, false) {
            Ok(Err(e)) if e[0].id == "QVS2301" => refused.push(e[0].to_string()),
            other => return Err(format!("{} shut down: {other:?}", kind.word())),
        }
    }
    said(partd(&mut guest)?.shut_down_as(id, partd::proto::Shutdown::Immediate, 0, false))?;
    manager::ended(&mut guest, id, run.deadline())?;
    Ok(format!(
        "QS/VS's own UKI as a guest: Running {after:.0} s after its activation, once the wait \
         for a channel was over, marked as having none, current code {}; delayed and \
         operating-system shut down refused ({}); immediate shut down ended it",
        s.code,
        refused.join("; ")
    ))
}
