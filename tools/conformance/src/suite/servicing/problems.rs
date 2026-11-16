// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Step 0's console that was away, and suites 2 to 4: problems.

use std::time::{Duration, Instant};

use qsi::any::{AnyEvent, AnyRequest};
use qsi::host::vres;
use qsi::service as s;
use qsvshook::proto::command;

use super::{Console, Opts, console, host, problem, problems, problems_of};
use crate::suite::base::{Run, channel};
use crate::suite::qsi::eventually;
use poold::Calls as _;

use crate::suite::storage::{disk_names, pool_until, poold, said};

/// Suite 1's last part: a console that was away while the server was
/// down gets the pools' and the partitions' changes from the log.
pub fn away(run: &Run<'_>) -> Result<String, String> {
    let mut rig = host(
        run,
        &Opts {
            pool_disks: 1,
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let names = disk_names(&mut rig.q.guest, 1)?;
    said(poold(&mut rig.q.guest)?.create_pool(
        "POOL1".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(&mut rig.q.guest, "POOL1", run.deadline(), |p| {
        !p.mount.is_empty()
    })?;
    // Everything said so far, and the last number.
    let start = c.subscribe(None)?;
    std::thread::sleep(Duration::from_secs(5));
    let seen = super::super::qsi::flows::gather(&mut c.link, Duration::from_secs(3))?;
    let last = seen.last().map_or(start.next - 1, |(n, _)| *n);
    c.link.close();
    // The server down, twice in a row so it stays down a while, and
    // while it is: a virtual disk made and a partition made.
    let started = super::super::qsi::posted(&mut rig.q, &refcode::QSI_STARTED);
    for _ in 0..2 {
        channel(&mut rig.q.guest)?.ask(&format!("{} qsid", command::KILL))?;
    }
    said(poold(&mut rig.q.guest)?.create_vdisk("POOL1".into(), "AWAY".into(), 64, false))?;
    let made = crate::suite::partition::manager::partd(&mut rig.q.guest)?;
    let mut made = made;
    let d = partd::Calls::create(&mut made, "AWAY".into())
        .map_err(|e| e.to_string())?
        .map_err(|e| crate::suite::partition::manager::refused(&e))?;
    super::super::qsi::posted_after(
        &mut rig.q,
        &refcode::QSI_STARTED,
        started,
        Duration::from_mins(1),
    )?;
    c.again(&rig.q, Duration::from_mins(1))?;
    let r = c.subscribe(Some(last))?;
    if r.replay != qsi::base::Replay::Resumed {
        return Err(format!("the console's resumed events: {r:?}"));
    }
    let mut vdisk = false;
    let mut defined = false;
    c.until(Duration::from_secs(30), |e| {
        match e {
            AnyEvent::Vres(vres::Event::Vdisk(v)) if v.name == "AWAY" && v.vdisk.is_some() => {
                vdisk = true;
            }
            AnyEvent::Partitions(qsi::host::partitions::Event::Defined(p))
                if p.id == d.id && p.exists =>
            {
                defined = true;
            }
            _ => {}
        }
        (vdisk && defined).then_some(())
    })?;
    c.link.close();
    Ok(format!(
        "a console that last saw event {last} resumed after the server was killed twice; the \
         virtual disk AWAY and partition {} made while it was down were in its events",
        d.id
    ))
}

/// Suite 2: a pulled pool disk opens one problem naming the unit by its
/// location code and the pool; the hot spare's rebuild clears it; the
/// spare, now the unit, pulled again opens a second; the first closed
/// over QSI with the console's user recorded.
#[allow(clippy::too_many_lines)] // one scenario, in order
pub fn pool(run: &Run<'_>) -> Result<String, String> {
    let mut rig = host(
        run,
        &Opts {
            pool_disks: 3,
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let names = disk_names(&mut rig.q.guest, 3)?;
    said(poold(&mut rig.q.guest)?.create_pool(
        "POOL1".into(),
        "mirrored".into(),
        names[..2].to_vec(),
        names[2..].to_vec(),
        Vec::new(),
    ))?;
    let within = Instant::now() + Duration::from_mins(10);
    pool_until(&mut rig.q.guest, "POOL1", within, |p| {
        p.state == poold::proto::state::ACTIVE
    })?;
    let file = "/pools/POOL1/pulled";
    let pull = |rig: &mut super::Rig<'_>, hd: usize| -> Result<(), String> {
        rig.q.guest.monitor(&format!("drive_del hd{hd}"))?;
        channel(&mut rig.q.guest)?.ask_within(
            &format!("copy /dev/urandom 0 {file} 0 {}", 4 << 20),
            Duration::from_mins(2),
        )?;
        Ok(())
    };
    // The installed disk is hd1; the pool's are hd2 to hd4.
    pull(&mut rig, 2)?;
    let first = problems_of(
        &mut c,
        refcode::POOL_UNIT_FAILED.code,
        1,
        Duration::from_mins(2),
    )?;
    let p1 = problem(&mut c, first[0].number)?;
    let unit = p1
        .resources
        .iter()
        .find(|r| r.kind == s::ResourceKind::DiskUnit)
        .ok_or_else(|| format!("problem {} names no disk unit: {p1:?}", first[0].number))?
        .clone();
    if unit.location.is_empty()
        || !p1
            .resources
            .iter()
            .any(|r| r.kind == s::ResourceKind::StoragePool && r.name == "POOL1")
    {
        return Err(format!(
            "problem {}'s resources: {:?}",
            first[0].number, p1.resources
        ));
    }
    // The spare taken and rebuilt: the first cleared, still open.
    pool_until(
        &mut rig.q.guest,
        "POOL1",
        Instant::now() + Duration::from_mins(15),
        |p| p.state == poold::proto::state::ACTIVE,
    )?;
    let cleared = eventually(Duration::from_mins(2), || {
        problem(&mut c, first[0].number)
            .ok()
            .filter(|p| !p.summary.holds)
    })
    .ok_or("the rebuild's end did not clear the first problem")?;
    if cleared.summary.status != s::ProblemStatus::Open {
        return Err("the cleared problem is not open".into());
    }
    // The spare, now in the pool, pulled: a second problem.
    pull(&mut rig, 4)?;
    let both = problems_of(
        &mut c,
        refcode::POOL_UNIT_FAILED.code,
        2,
        Duration::from_mins(2),
    )?;
    let second = both
        .iter()
        .find(|p| p.number != first[0].number)
        .ok_or("no second problem")?
        .clone();
    // Closed over QSI: the console's user.
    let closed = match c.ask(AnyRequest::Service(s::Request::Close(s::Close {
        number: first[0].number,
        anyway: false,
    })))? {
        qsi::any::AnyReply::Service(s::Reply::Close(x)) => x,
        other => return Err(format!("{other:?}")),
    };
    if closed.status != s::ProblemStatus::Closed || closed.closed_by.is_empty() {
        return Err(format!("the close: {closed:?}"));
    }
    let again = c.refused(&AnyRequest::Service(s::Request::Close(s::Close {
        number: first[0].number,
        anyway: false,
    })))?;
    c.link.close();
    Ok(format!(
        "pulled unit at {} opened problem {} ({}: {}); the hot spare's rebuild cleared it; the \
         spare pulled again opened problem {}; problem {} closed over QSI by {:?}, a second close \
         refused {}",
        unit.location,
        first[0].number,
        first[0].refcode.code,
        first[0].description,
        second.number,
        closed.number,
        closed.closed_by,
        again.join(" ")
    ))
}

/// Suite 3: each condition the harness can cause opens its problem with
/// the right code, description and resources.
pub fn conditions(run: &Run<'_>) -> Result<String, String> {
    let mut rig = host(
        run,
        &Opts {
            hooks: vec!["hold-memory"],
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let mut lines = Vec::new();
    let mut check = |c: &mut Console,
                     code: &refcode::RefCode,
                     kind: Option<s::ResourceKind>|
     -> Result<(), String> {
        let found = problems_of(c, code.code, 1, Duration::from_mins(3))?;
        let p = problem(c, found[0].number)?;
        let want = qvsmsg::problems::condition(code.code)
            .and_then(|c| c.opens)
            .ok_or_else(|| format!("{} opens no problem", code.code))?;
        if p.summary.msgid != want.message.id {
            return Err(format!(
                "{}: message {}, not {}",
                code.code, p.summary.msgid, want.message.id
            ));
        }
        if let Some(k) = kind
            && !p.resources.iter().any(|r| r.kind == k)
        {
            return Err(format!("{}: resources {:?}", code.code, p.resources));
        }
        lines.push(format!(
            "{} problem {} `{}`",
            code.code, p.summary.number, p.summary.description
        ));
        Ok(())
    };
    // The test image's flaky service, stopped after its retries.
    check(
        &mut c,
        &refcode::SERVICE_STOPPED,
        Some(s::ResourceKind::System),
    )?;
    // Fewer huge pages than configured: the hold-memory hook.
    check(&mut c, &refcode::MEMORY_SHORT, None)?;
    // Repeated failed pairings: a password's attempts spent.
    crate::suite::qsi::to_pairing(&mut rig.q, run.deadline())?;
    let (good, _) = crate::suite::qsi::start_pairing(&mut rig.q, run.deadline())?;
    let wrong: String = good.chars().rev().collect();
    for n in 0..5 {
        let id = crate::suite::qsi::identity(&format!("BAD{n}"))?;
        let _ = crate::suite::qsi::pair(&rig.q, &id, &wrong, "BAD");
    }
    check(&mut c, &refcode::QSI_PAIRING_FAILING, None)?;
    let _ = rig.q.guest.type_keys(crate::qemu::term::key::F3);
    // /data nearly full, then room again: cleared.
    let room = data_free(&mut rig)?;
    let filled = room.saturating_sub(room / 20 + (8 << 20));
    channel(&mut rig.q.guest)?.ask_within(
        &format!("copy /dev/zero 0 /data/fill 0 {filled}"),
        Duration::from_mins(5),
    )?;
    check(&mut c, &refcode::DATA_NEARLY_FULL, None)?;
    channel(&mut rig.q.guest)?.ask(&format!("{} /data/fill", command::RM))?;
    let full = problems_of(
        &mut c,
        refcode::DATA_NEARLY_FULL.code,
        1,
        Duration::from_secs(5),
    )?;
    eventually(Duration::from_mins(3), || {
        problem(&mut c, full[0].number)
            .ok()
            .filter(|p| !p.summary.holds)
    })
    .ok_or("the room again did not clear the problem")?;
    lines.push(format!("{} cleared it", refcode::DATA_HAS_ROOM.code));
    c.link.close();
    Ok(format!(
        "{}; the conditions the harness cannot cause are the table's unit tests (problem::book, problem::tests)",
        lines.join("; ")
    ))
}

/// The bytes free on `/data`, by the inventory's view of the data file
/// system: what the hook's `df` of it says.
fn data_free(rig: &mut super::Rig<'_>) -> Result<u64, String> {
    let lines = channel(&mut rig.q.guest)?.ask("statfs /data")?;
    lines
        .iter()
        .find_map(|l| l.strip_prefix("free ").and_then(|v| v.trim().parse().ok()))
        .ok_or_else(|| format!("statfs: {lines:?}"))
}

/// Suite 4: the problem service killed while codes are posted: none is
/// lost and none is doubled.
pub fn kill(run: &Run<'_>) -> Result<String, String> {
    const POSTS: u32 = 60;
    let mut rig = host(run, &Opts::default())?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let code = &refcode::CLOCK_NOT_SET;
    let mut kills = 0;
    for n in 0..POSTS {
        channel(&mut rig.q.guest)?.ask(&format!("{} {} burst {n}", command::POST, code.code))?;
        if n % 15 == 7 {
            channel(&mut rig.q.guest)?.ask(&format!("{} probd", command::KILL))?;
            kills += 1;
        }
    }
    let found = eventually(Duration::from_mins(2), || {
        let of: Vec<_> = problems(&mut c)
            .ok()?
            .into_iter()
            .filter(|p| p.refcode.code == code.code)
            .collect();
        (of.len() == 1 && of[0].count >= POSTS).then_some(of)
    })
    .ok_or_else(|| {
        let of: Vec<String> = problems(&mut c)
            .unwrap_or_default()
            .iter()
            .filter(|p| p.refcode.code == code.code)
            .map(|p| format!("problem {} count {}", p.number, p.count))
            .collect();
        format!("{POSTS} postings, the service killed {kills} times: {of:?}")
    })?;
    if found[0].count != POSTS {
        return Err(format!("{POSTS} postings counted {} times", found[0].count));
    }
    c.link.close();
    Ok(format!(
        "{POSTS} postings of {} with the problem service killed {kills} times among them: one \
         problem ({}), counted {} times",
        code.code, found[0].number, found[0].count
    ))
}
