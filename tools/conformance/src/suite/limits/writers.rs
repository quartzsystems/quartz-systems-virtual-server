// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `power-writers`: the power pulled thirty times at random moments
//! while every writer of `/data` that the earlier pulls did not cover is
//! busy at once (the `writers` hook: definitions, a switch, a pool's
//! virtual disks, problems, host dumps, and the QSI server's event log
//! behind them). After each IPL every service is ready and has not
//! failed, nothing posted a state file not valid or `/data` not mounted,
//! and each service answers with what the writers made.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use partd::Calls as _;
use poold::proto::Calls as _;
use qsvshook::writers::{PARTITIONS, POOL, SWITCH};
use vswitchd::proto::Calls as _;

use crate::qemu::session::Session;
use crate::suite::base::power::Moments;
use crate::suite::base::{Run, channel};
use crate::suite::network::vswitchd;
use crate::suite::partition::manager::partd;
use crate::suite::partition::{fresh_host, host, launch};
use crate::suite::storage::{POOL_DISK_BYTES, disk_names, pool_disks, pool_until, poold};

/// How often the power is pulled.
const CUTS: u32 = 30;

/// What the host has after an IPL, its base ready (`host` waited for
/// it): an error, or how many partitions, virtual disks and host dumps
/// the writers left.
fn whole(g: &mut Session, run: &Run<'_>) -> Result<(usize, usize, usize), String> {
    for code in [&refcode::STATE_INVALID, &refcode::DATA_NOT_MOUNTED] {
        if g.said(&code.tag()) {
            return Err(format!("the IPL posted {}", code.line()));
        }
    }
    let status = channel(g)?.status()?;
    if let Some(s) = status
        .services
        .iter()
        .find(|s| s.failures > 0 || s.state != "ready")
    {
        return Err(format!(
            "{} is {} with {} failure(s)",
            s.name, s.state, s.failures
        ));
    }
    let partitions = partd(g)?
        .list()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|p| PARTITIONS.contains(&p.name.as_str()))
        .count();
    vswitchd(g)?.switches().map_err(|e| e.to_string())?;
    pool_until(g, POOL, run.deadline(), |p| !p.mount.is_empty())?;
    let vdisks = poold(g)?
        .vdisks(POOL.into())
        .map_err(|e| e.to_string())?
        .len();
    let dumps = channel(g)?.ls("/data/dumps/host").map_or(0, |l| l.len());
    Ok((partitions, vdisks, dumps))
}

/// The suite.
pub fn power_writers(run: &Run<'_>) -> Result<String, String> {
    fresh_host(run)?;
    let disks = pool_disks(run, 1, POOL_DISK_BYTES)?;
    let mut l = launch(run, &disks)?;
    l.snapshot = false;
    l.monitor = Some(run.socket("monitor.sock"));
    l.oem.push(qsvshook::oem_string(qsvshook::WRITERS));
    let mut guest = host(run, &l)?;
    let names = disk_names(&mut guest, 1)?;
    crate::suite::storage::said(poold(&mut guest)?.create_pool(
        POOL.into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| u64::from(d.subsec_nanos()) | 1);
    let mut moments = Moments(seed);
    let mut last = (0, 0, 0);
    let mut switch_seen = false;
    for cut in 0..=CUTS {
        if cut > 0 {
            guest = host(run, &l)?;
        }
        let at = |e: String| format!("after {cut} power cuts: {e}");
        last = whole(&mut guest, run).map_err(at)?;
        switch_seen |= vswitchd(&mut guest)?
            .switches()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|s| s.switch.as_ref().is_some_and(|w| w.name == SWITCH));
        if cut < CUTS {
            // The writers run on for two to six seconds.
            std::thread::sleep(Duration::from_millis(2000 + moments.number() % 4000));
            guest.kill();
        }
    }
    if last.0 == 0 || !switch_seen {
        return Err(format!(
            "the writers left {last:?} and the switch {switch_seen}: they did not run"
        ));
    }
    Ok(format!(
        "{CUTS} power cuts with every writer busy: each IPL had every service ready and \
         none failed, no state file refused, /data mounted; at the end {} partitions, {} \
         virtual disks, {} host dumps",
        last.0, last.1, last.2
    ))
}
