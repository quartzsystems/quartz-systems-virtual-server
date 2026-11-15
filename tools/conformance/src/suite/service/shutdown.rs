// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Shut down asked of the system: delayed, operating system, forced
//! when it does not end in time; the host's own power off through each
//! system; auto start waiting for an IPL to complete.

use std::time::{Duration, Instant};

use partd::proto::{Posted, Shutdown, Source};
use partd::{Calls as _, State};
use qsvshook::proto::command;

use super::{history, ipl_patience, qs400_spec, set_service, system_codes, up};
use crate::qemu::image::sparse_copy;
use crate::suite::base::{Run, base_ready, channel, posted};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::partition::{
    QS400_MEMORY_MIB, QSVS_MEMORY_MIB, fresh_host, host, launch, qs400_image, signal_to, unit,
};

/// The side a history's last activation read in the LIC record.
fn side_read(history: &[Posted]) -> Option<String> {
    history
        .iter()
        .rev()
        .find(|p| p.code == refcode::PARTITION_RECORD_READ.code)
        .and_then(|p| p.text.split("side ").nth(1))
        .and_then(|s| s.get(..1))
        .map(str::to_string)
}

/// Suite 4: a delayed shut down with a delay: the system ends its jobs
/// and powers down, Not activated with the `D` code in its history;
/// with restart it comes back on the recorded side.
pub fn delayed(run: &Run<'_>) -> Result<String, String> {
    let mut guest = super::qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    let spec = qs400_spec(None);
    up(&mut guest, id, &spec, ipl_patience(run))?;
    let asked = Instant::now();
    said(partd(&mut guest)?.shut_down_as(id, Shutdown::Delayed, 10, false))?;
    let shutting = manager::status(&mut guest, id)?.state;
    let s = manager::ended(&mut guest, id, Instant::now() + 3 * run.timeout)?;
    let took = asked.elapsed().as_secs_f64();
    let h = history(&mut guest, id)?;
    let codes = system_codes(&h);
    if s.state != State::NotActivated || !codes.iter().any(|c| c == "D1001090") {
        return Err(format!("after the shut down: {} {codes:?}", s.state.word()));
    }
    if shutting != State::ShuttingDown
        || !h.iter().any(|p| p.code == refcode::SHUTDOWN_DELAYED.code)
    {
        return Err(format!("while it went: {}", shutting.word()));
    }
    // With restart: activated again on the side the record names.
    up(&mut guest, id, &spec, ipl_patience(run))?;
    let side = side_read(&history(&mut guest, id)?);
    let before = history(&mut guest, id)?.len();
    said(partd(&mut guest)?.shut_down_as(id, Shutdown::Delayed, 10, true))?;
    let deadline = ipl_patience(run);
    loop {
        let h = history(&mut guest, id)?;
        let restarted = h
            .iter()
            .skip(before.saturating_sub(10))
            .any(|p| p.code == refcode::PARTITION_RESTART.code);
        let s = manager::status(&mut guest, id)?;
        if restarted
            && s.state == State::Running
            && h.iter().rev().take(40).any(|p| p.code == "C6001090")
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "not back after the restart: {} {}",
                s.state.word(),
                s.code
            ));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let again = side_read(&history(&mut guest, id)?);
    if again != side {
        return Err(format!("side {side:?} before the restart, {again:?} after"));
    }
    Ok(format!(
        "DELAY(10): Shutting down, then Not activated after {took:.0} s with D1001090 in the \
         history; with restart, C2005110 and activated again on side {} as recorded, its IPL \
         complete",
        side.unwrap_or_default()
    ))
}

/// Suite 5: an operating-system shut down; and a system made to ignore
/// the request is forced after the timeout, with a message that says
/// so. The system is made to ignore it by its VMM being stopped as soon
/// as it accepted.
pub fn system(run: &Run<'_>) -> Result<String, String> {
    let mut guest = super::qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    let spec = qs400_spec(None);
    up(&mut guest, id, &spec, ipl_patience(run))?;
    let asked = Instant::now();
    said(partd(&mut guest)?.shut_down_as(id, Shutdown::System, 0, false))?;
    manager::ended(&mut guest, id, Instant::now() + 3 * run.timeout)?;
    let took = asked.elapsed().as_secs_f64();
    let codes = system_codes(&history(&mut guest, id)?);
    if !codes.iter().any(|c| c == "D1001090") {
        return Err(format!("no D1001090: {codes:?}"));
    }
    set_service(&mut guest, |s| s.system_timeout_secs = 10)?;
    up(&mut guest, id, &spec, ipl_patience(run))?;
    said(partd(&mut guest)?.shut_down_as(id, Shutdown::System, 0, false))?;
    signal_to(&mut guest, id, "STOP")?;
    let stopped = Instant::now();
    let s = manager::ended(&mut guest, id, Instant::now() + Duration::from_secs(120))?;
    let forced = stopped.elapsed().as_secs_f64();
    let message = s.message.ok_or("no message")?;
    if message.id != "QVS2305" {
        return Err(format!("the message: {message}"));
    }
    if !history(&mut guest, id)?
        .iter()
        .any(|p| p.code == refcode::SHUTDOWN_FORCED.code)
    {
        return Err("no A2009040 in the history".into());
    }
    Ok(format!(
        "OPTION(*IMMED): Not activated after {took:.0} s with D1001090; with the timeout at 10 s \
         and the system stopped once it accepted, forced {forced:.0} s later: A2009040 and \
         `{message}`"
    ))
}

/// Two QS/400 partitions' units, and a host with room for both.
fn two_qs400<'a>(run: &Run<'a>) -> Result<(crate::suite::base::Guest<'a>, [u32; 2]), String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let a = unit(run, &image)?;
    let b = run.file("unit2.img");
    sparse_copy(&image, &b)?;
    let mut l = launch(run, &[a, b])?;
    l.memory_mb = 8192;
    let mut guest = host(run, &l)?;
    let first = manager::partition(&mut guest, "FIRST")?;
    let second = manager::partition(&mut guest, "SECOND")?;
    Ok((guest, [first, second]))
}

fn qs400_on(device: &str, name: &str) -> Spec {
    let mut spec = qs400_spec(None);
    spec.backing = vec![device.to_string()];
    spec.name = name.into();
    spec.memory_mib = (QS400_MEMORY_MIB, QS400_MEMORY_MIB);
    spec
}

/// The codes of a history's last IPL of the system, from C1001000.
fn last_ipl(history: &[Posted]) -> Vec<String> {
    let codes: Vec<String> = system_codes(history);
    let from = codes.iter().rposition(|c| c == "C1001000").unwrap_or(0);
    codes[from..].to_vec()
}

/// Suite 16: the host's power off shuts the partitions down through
/// their systems in reverse start order within the timeout, and the
/// next IPLs of their systems are clean.
pub fn host_power_off(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = two_qs400(run)?;
    for (id, (device, name)) in ids
        .iter()
        .zip([("/dev/vdb", "FIRST"), ("/dev/vdc", "SECOND")])
    {
        up(&mut guest, *id, &qs400_on(device, name), ipl_patience(run))?;
    }
    let began = Instant::now();
    channel(&mut guest)?.tell(command::POWER_OFF)?;
    let limit = Instant::now() + Duration::from_mins(15);
    // The last activated first, each through its system.
    guest.expect(&posted(&refcode::SHUTDOWN_SYSTEM, "SECOND"), limit)?;
    guest.expect(&posted(&refcode::PARTITION_POWERED_OFF, "SECOND"), limit)?;
    guest.expect(&posted(&refcode::SHUTDOWN_SYSTEM, "FIRST"), limit)?;
    guest.expect(&posted(&refcode::PARTITION_POWERED_OFF, "FIRST"), limit)?;
    guest.expect(&posted(&refcode::PARTITIONS_ENDED, ""), limit)?;
    guest.expect(&posted(&refcode::POWER_OFF, ""), limit)?;
    guest.wait_exit(limit)?;
    let took = began.elapsed().as_secs_f64();
    if guest.said(&posted(&refcode::SHUTDOWN_FORCED, "")) {
        return Err("a partition's shut down was forced".into());
    }
    // The host again: each system IPLs clean.
    let mut l = launch(run, &[run.file("unit.img"), run.file("unit2.img")])?;
    l.memory_mb = 8192;
    let mut guest = host(run, &l)?;
    let mut clean = Vec::new();
    for (id, (device, name)) in ids
        .iter()
        .zip([("/dev/vdb", "FIRST"), ("/dev/vdc", "SECOND")])
    {
        up(&mut guest, *id, &qs400_on(device, name), ipl_patience(run))?;
        let codes = last_ipl(&history(&mut guest, *id)?);
        if codes
            .iter()
            .any(|c| c.starts_with("B1") || c == "C1001085" || c == "C1001087")
        {
            return Err(format!("{name}'s next IPL: {codes:?}"));
        }
        clean.push(format!("{name} {} codes", codes.len()));
    }
    Ok(format!(
        "the host's power off ended SECOND then FIRST, each through its system (C2009041, \
         C2005100), none forced, the host off after {took:.0} s; the next IPLs were clean: {}",
        clean.join(", ")
    ))
}

/// Suite 17: auto start's wait for an IPL to complete orders two
/// partitions.
pub fn auto_wait(run: &Run<'_>) -> Result<String, String> {
    let image = qs400_image()?;
    let release = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    fresh_host(run)?;
    let a = unit(run, &image)?;
    let b = run.file("cheap.img");
    sparse_copy(release, &b)?;
    let units = [a, b];
    let mut guest = host(run, &launch(run, &units)?)?;
    let first = manager::partition(&mut guest, "FIRST")?;
    let second = manager::partition(&mut guest, "SECOND")?;
    manager::put(&mut guest, first, &qs400_on("/dev/vdb", "NORMAL"))?;
    let mut cheap = Spec::new(1, QSVS_MEMORY_MIB, "/dev/vdc");
    cheap.side = model::IplSource::A;
    cheap.name = "NORMAL".into();
    manager::put(&mut guest, second, &cheap)?;
    for (id, order, wait) in [(first, 1, true), (second, 2, false)] {
        let mut d = said(partd(&mut guest)?.definition(id))?;
        d.auto_start = model::AutoStart {
            enabled: true,
            order,
            delay_secs: 0,
            wait,
            wait_secs: 1800,
        };
        said(partd(&mut guest)?.change(d))?;
    }
    channel(&mut guest)?.tell(command::RESTART)?;
    base_ready(&mut guest, Instant::now() + 3 * run.timeout)?;
    let deadline = ipl_patience(run);
    manager::until(&mut guest, first, State::Running, deadline)?;
    manager::until(
        &mut guest,
        second,
        State::Running,
        Instant::now() + 3 * run.timeout,
    )?;
    let a = history(&mut guest, first)?;
    let b = history(&mut guest, second)?;
    let complete = a
        .iter()
        .rev()
        .find(|p| p.code == "C6001090" && p.source == Source::System)
        .ok_or("FIRST has no C6001090")?
        .time;
    let activated = b
        .iter()
        .rev()
        .find(|p| p.code == refcode::PARTITION_ACTIVATING.code)
        .ok_or("SECOND was not activated")?
        .time;
    if activated < complete {
        return Err(format!(
            "SECOND was activated at {activated}, before FIRST's IPL complete at {complete}"
        ));
    }
    Ok(format!(
        "auto start with the wait: FIRST's IPL complete at {complete}, SECOND activated at \
         {activated}, {} s after",
        activated - complete
    ))
}
