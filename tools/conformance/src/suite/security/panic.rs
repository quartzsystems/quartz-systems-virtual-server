// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A host whose kernel panics (`docs/SECURITY.md`): it restarts by
//! itself; its kernel's last messages, kept in the firmware's variables
//! across the restart, are moved to `/data/log/panics`; `A2001005` is
//! posted with why it panicked and opens its problem; and the next host
//! dump holds the messages. The panic is LKDTM's, a test image's
//! kernel's alone.

use std::time::{Duration, Instant};

use qsictl::service::HostDump;
use qsvshook::proto::command;

use crate::suite::base::{Run, channel, posted};
use crate::suite::servicing::{Opts, console, host};

/// LKDTM's way in.
const PROVOKE: &str = "/sys/kernel/debug/provoke-crash/DIRECT";
/// What the kernel says LKDTM's panic is.
const WHY: &str = "dumptest";

/// The suite.
pub fn panicked(run: &Run<'_>) -> Result<String, String> {
    let mut rig = host(run, &Opts::default())?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    {
        let ch = channel(&mut rig.q.guest)?;
        ch.ask(command::DEBUGFS)?;
        // The host goes before it can answer.
        let _ = ch.ask(&format!("{} {PROVOKE} PANIC", command::WRITE));
    }
    rig.restarted(run, Duration::from_mins(2))?;
    let line = rig
        .q
        .guest
        .line_with(&refcode::HOST_PANICKED.tag(), run.deadline())?;
    if !line.contains(WHY) {
        return Err(format!("the panic was posted without why: {line}"));
    }
    rig.q.guest.seen(
        &posted(&refcode::HOST_PANICKED, ""),
        Instant::now() + Duration::from_secs(5),
    )?;
    let ch = channel(&mut rig.q.guest)?;
    let kept = ch.ls("/data/log/panics")?;
    if kept.is_empty() {
        return Err("nothing in /data/log/panics".into());
    }
    let pstore = ch.ls("/sys/fs/pstore")?;
    if !pstore.is_empty() {
        return Err(format!("the firmware's records were left: {pstore:?}"));
    }
    let deadline = Instant::now() + Duration::from_mins(1);
    loop {
        let book = ch.text("/data/problems/problems.json").unwrap_or_default();
        if book.contains(refcode::HOST_PANICKED.code) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("no problem was opened for A2001005".into());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    // The next host dump holds what was kept.
    c.again(&rig.q, Duration::from_mins(2))?;
    qsictl::service::host_dump(&mut c.link, HostDump::Take)?;
    let reply = c.ask(qsi::any::AnyRequest::Base(qsi::base::Request::Dumps(
        qsi::base::ListDumps {},
    )))?;
    let qsi::any::AnyReply::Base(qsi::base::Reply::Dumps(all)) = reply else {
        return Err(format!("{reply:?}"));
    };
    let dump = all.last().ok_or("no host dump listed")?.clone();
    let file = run.file("panic-hostdump.zip");
    qsictl::service::host_dump(
        &mut c.link,
        HostDump::Fetch {
            name: dump.name.clone(),
            file: file.clone(),
        },
    )?;
    c.link.close();
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    let entries = hostdump::zip::entries(&bytes)?;
    let in_dump: Vec<&(String, Vec<u8>)> = entries
        .iter()
        .filter(|(n, _)| n.starts_with("log/panics/"))
        .collect();
    if !in_dump
        .iter()
        .any(|(_, d)| String::from_utf8_lossy(d).contains(WHY))
    {
        return Err(format!(
            "the host dump holds no panic record saying `{WHY}`: {:?}",
            in_dump.iter().map(|(n, _)| n).collect::<Vec<_>>()
        ));
    }
    Ok(format!(
        "LKDTM's panic: the host restarted by itself; `{}`; {} record(s) in \
         /data/log/panics, none left in the firmware; its problem opened; host dump {} holds {}",
        line.trim(),
        kept.len(),
        dump.name,
        in_dump.len()
    ))
}
