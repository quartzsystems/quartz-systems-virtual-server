// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 6: supervision. A service killed by the test hook is
//! restarted; one that keeps failing is stopped after five tries with
//! the `A2` code, and the host stays up.

use std::time::{Duration, Instant};

use qsvshook::proto::command;
use release::{fact, screen};

use super::{Run, all_ready, booted, channel, fact_of, posted, screen_with};
use crate::qemu::Launch;

/// The failures in ten minutes that end the restarts.
const TRIES: u32 = 5;

/// A killed service comes back.
fn restart(run: &Run<'_>) -> Result<String, String> {
    let mut guest = booted(run, &run.launch())?;
    let channel = channel(&mut guest)?;
    let before = channel.status()?;
    all_ready(&before)?;
    let pid = before.service("netd").map_or(0, |s| s.pid);
    channel.ask(&format!("{} netd", command::KILL))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let after = channel.until("netd started again and ready", deadline, |s| {
        s.service("netd")
            .is_some_and(|n| n.state == "ready" && n.pid != pid && n.failures == 1)
    })?;
    // Nothing else was disturbed.
    all_ready(&after)?;
    let others: u32 = after
        .services
        .iter()
        .filter(|s| s.name != "netd")
        .map(|s| s.failures)
        .sum();
    if others != 0 {
        return Err(format!("other services failed too: {:?}", after.services));
    }
    let now = after.service("netd").map_or(0, |s| s.pid);
    Ok(format!(
        "netd killed (process {pid}) and ready again (process {now})"
    ))
}

/// A service that fails every time is given up on; the host goes on.
fn give_up(run: &Run<'_>) -> Result<String, String> {
    let launch = Launch {
        oem: vec![qsvshook::oem_string(qsvshook::FLAKY_FAILS)],
        ..run.launch()
    };
    let mut guest = run.boot(&launch)?;
    let deadline = run.deadline();
    let code = posted(&refcode::SERVICE_STOPPED, "flaky");
    guest.seen(&code, deadline)?;
    screen_with(
        &mut guest,
        &[screen::service(
            "flaky",
            &format!("stopped, {TRIES} failures"),
        )],
        deadline,
    )?;
    let status = guest.channel(deadline)?.status()?;
    let flaky = status.service("flaky").ok_or("no flaky in the status")?;
    if (flaky.state.as_str(), flaky.failures, flaky.pid) != ("stopped", TRIES, 0) {
        return Err(format!("flaky is {flaky:?}"));
    }
    // The host stays up: the other services are ready and untouched,
    // and PID 1 goes on running.
    for service in status.services.iter().filter(|s| s.name != "flaky") {
        if service.state != "ready" || service.failures != 0 {
            return Err(format!("{service:?} beside the failing service"));
        }
    }
    if fact_of(&status, fact::READY)? != "no" {
        return Err("the base says it is ready with a service stopped".into());
    }
    let reference = fact_of(&status, fact::REFERENCE)?;
    if !reference.starts_with(refcode::SERVICE_STOPPED.code) {
        return Err(format!("the current reference code is `{reference}`"));
    }
    let heartbeat = refcode::HEARTBEAT.tag();
    guest.expect(&code, deadline)?;
    guest.expect(&heartbeat, deadline)?;
    guest.expect(&heartbeat, deadline)?;
    Ok(format!(
        "flaky stopped after {TRIES} failures with `{code}`; the host runs on"
    ))
}

/// The suite.
pub fn supervision(run: &Run<'_>) -> Result<String, String> {
    Ok(format!("{}; {}", restart(run)?, give_up(run)?))
}
