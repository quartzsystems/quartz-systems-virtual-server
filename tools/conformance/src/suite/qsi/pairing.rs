// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 2 and 3: pairing from Service Tools, and who may connect.

use std::time::Duration;

use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::{base, pair};
use qsictl::link::Link;
use qsitrust::Binding;

use super::{
    PAIRING, Qsi, eventually, host, identity, link, masked, pair, start_pairing, to_pairing,
};
use crate::qemu::term::key;
use crate::suite::base::{Run, channel};
use crate::suite::install::{golden, option, titled};

/// A pairing attempt that must not pin: refused at the handshake (no
/// password out), or by the host's message.
fn refused_pairing(q: &Qsi<'_>, password: &str, name: &str) -> Result<String, String> {
    let id = identity(name)?;
    match pair(q, &id, password, name) {
        Ok(_) => Err(format!("{name} paired, and must not have")),
        Err(e) => Ok(e),
    }
}

/// The codes posted so far that are `code`.
fn count(q: &mut Qsi<'_>, code: &refcode::RefCode) -> usize {
    q.guest.drain();
    q.guest.log.matches(&code.tag()).count()
}

/// Suite 2: the password pairs once; a second use, a wrong password
/// five times and an expired password are each refused, with the codes.
pub fn pairing(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, false)?;
    let deadline = run.deadline();
    let menu = to_pairing(&mut q, deadline)?;
    golden("tools-qsi", &menu)?;
    let (password, screen) = start_pairing(&mut q, deadline)?;
    golden("tools-qsi-pair", &masked(&screen))?;
    if count(&mut q, &refcode::QSI_PAIRING_STARTED) != 1 {
        return Err("no C2010020 when the password was made".into());
    }

    // It pairs once.
    let id = identity("MC1")?;
    let host_print = pair(&q, &id, &password, "MC1")?.fingerprint;
    q.guest.seen(&refcode::QSI_PAIRED.tag(), deadline)?;
    let mut l = link(&q, &id, host_print)?;
    let me = l.ask(&AnyRequest::Base(base::Request::Identify(Empty)))?;
    let Ok(Some(AnyReply::Base(base::Reply::Identify(me)))) = me.decode() else {
        return Err("identify was not answered once paired".into());
    };
    l.close();
    // Its screen says so: the password is gone.
    q.guest.type_keys(key::F5)?;
    q.guest.screen("the password gone", deadline, |g| {
        g.contains("The password is gone")
    })?;
    let second = refused_pairing(&q, &password, "MC2")?;
    q.guest.type_keys(key::F12)?;
    titled(&mut q.guest, PAIRING, deadline)?;

    // Five wrong passwords: each refused, then the password is gone.
    let (good, _) = start_pairing(&mut q, deadline)?;
    let mut wrongs = Vec::new();
    for n in 0..5 {
        let wrong: String = good.chars().rev().collect();
        let e = refused_pairing(&q, &wrong, &format!("BAD{n}"))?;
        wrongs.push(e);
    }
    let failed = eventually(Duration::from_secs(10), || {
        (count(&mut q, &refcode::QSI_PAIRING_FAILED) >= 5).then_some(())
    });
    if failed.is_none() {
        return Err(format!(
            "{} A2010030 for five wrong passwords: {wrongs:?}",
            count(&mut q, &refcode::QSI_PAIRING_FAILED)
        ));
    }
    q.guest.seen(&refcode::QSI_PAIRING_ENDED.tag(), deadline)?;
    let after = refused_pairing(&q, &good, "MC3")?;
    q.guest.type_keys(key::F12)?;
    titled(&mut q.guest, PAIRING, deadline)?;

    // A password that expires: the server started again with a life of
    // five seconds (a test image's), the password left unused past it.
    channel(&mut q.guest)?.ask(&format!(
        "{} /run/qsvshook.pairing-secs {}",
        qsvshook::proto::command::PUT,
        hex::encode(b"5")
    ))?;
    let started = count(&mut q, &refcode::QSI_STARTED);
    channel(&mut q.guest)?.ask(&format!("{} qsid", qsvshook::proto::command::KILL))?;
    eventually(Duration::from_mins(1), || {
        (count(&mut q, &refcode::QSI_STARTED) > started).then_some(())
    })
    .ok_or("the server was not started again")?;
    let ended = count(&mut q, &refcode::QSI_PAIRING_ENDED);
    let (late, _) = start_pairing(&mut q, deadline)?;
    std::thread::sleep(Duration::from_secs(7));
    let expired = refused_pairing(&q, &late, "MC4")?;
    eventually(Duration::from_secs(10), || {
        (count(&mut q, &refcode::QSI_PAIRING_ENDED) > ended).then_some(())
    })
    .ok_or("no C2010040 for the password that expired")?;
    q.guest.type_keys(key::F12)?;
    titled(&mut q.guest, PAIRING, deadline)?;
    let list = option(&mut q.guest, 1, "Work with Paired Consoles", deadline)?;
    if !list.contains("MC1") || list.contains("MC2") || list.contains("BAD") {
        return Err(format!("the consoles paired:\n{list}"));
    }
    Ok(format!(
        "{} paired once with the password shown, `{}` answered pinned; a second use refused \
         ({second}); five wrong passwords refused with five A2010030 and C2010040, the right \
         one refused after them ({after}); a password left past its five seconds refused \
         ({expired}) with C2010040",
        me.name, me.release
    ))
}

/// A relay: it ends TLS on each leg with a certificate of its own and
/// passes the console's messages through, knowing the password. The
/// host must refuse it.
fn relayed(q: &Qsi<'_>, password: &str) -> Result<String, String> {
    let relay = identity("RELAY")?;
    let victim = identity("VICTIM")?;
    let mut to_host = Link::connect(&q.address, &relay, None)?;
    // What the console's own leg to the relay would bind.
    let console_leg = Binding {
        mine: victim.fingerprint(),
        peer: relay.fingerprint(),
        exporter: [0x5a; 32],
    };
    let (console, start) = qsitrust::pair::Console::start(password, console_leg, "VICTIM")
        .map_err(|e| e.to_string())?;
    let reply = to_host.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: start.spake,
        name: start.name,
    })))?;
    let Ok(Some(pair::Reply::Answer(a))) = reply.typed::<pair::Reply>() else {
        return Ok(format!(
            "its start refused: {}",
            qsictl::pair::refusal(&reply)
        ));
    };
    if console
        .answer(&qsitrust::pair::Answer {
            spake: a.spake,
            confirm: a.confirm,
            name: a.name,
        })
        .is_ok()
    {
        return Err("the console accepted a relayed answer".into());
    }
    // The relay can only confirm with what it has, which is nothing.
    let confirm = to_host.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
        confirm: vec![0; 32],
    })))?;
    to_host.close();
    match confirm.refusal() {
        Some((m, _)) if m == qsi::msgid::PAIRING_FAILED => Ok(m.to_string()),
        other => Err(format!("the relayed confirmation: {other:?}")),
    }
}

/// Suite 3: an unpinned peer is refused with no pairing out; a pinned
/// console reconnects without pairing; a removed console is refused and
/// its session closed; a third pairing is refused; a relayed pairing
/// fails.
pub fn trust(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, false)?;
    let deadline = run.deadline();
    let stranger = identity("STRANGER")?;
    let Err(unpinned) = Link::connect(&q.address, &stranger, None) else {
        return Err("an unpinned console had a session with no pairing out".into());
    };
    to_pairing(&mut q, deadline)?;
    let (password, _) = start_pairing(&mut q, deadline)?;
    let relay = relayed(&q, &password)?;
    q.guest.type_keys(key::F12)?;
    titled(&mut q.guest, PAIRING, deadline)?;
    // The relay's attempt counted; a fresh password for the consoles.
    let (password, _) = start_pairing(&mut q, deadline)?;
    let a = identity("MC-A")?;
    let host_print = pair(&q, &a, &password, "MC-A")?.fingerprint;
    q.guest.type_keys(key::F12)?;
    titled(&mut q.guest, PAIRING, deadline)?;
    let (password, _) = start_pairing(&mut q, deadline)?;
    let b = identity("MC-B")?;
    pair(&q, &b, &password, "MC-B")?;
    q.guest.type_keys(key::F12)?;
    titled(&mut q.guest, PAIRING, deadline)?;
    // A third: Service Tools makes no password.
    q.guest.type_keys(&format!("2{}", key::ENTER))?;
    let third = q.guest.screen("the third pairing refused", deadline, |g| {
        g.contains("Two consoles are paired")
    })?;
    let _ = third;
    // Pinned: a session without pairing, again and again.
    for _ in 0..3 {
        let mut l = link(&q, &a, host_print)?;
        let r = l.ask(&AnyRequest::Base(base::Request::State(Empty)))?;
        if r.refusal().is_some() {
            return Err(format!("state refused: {}", qsictl::pair::refusal(&r)));
        }
        l.close();
    }
    // Remove A while it has a session: the session ends, and it is
    // refused after.
    let mut held = link(&q, &a, host_print)?;
    let list = option(&mut q.guest, 1, "Work with Paired Consoles", deadline)?;
    let rows: Vec<&str> = list
        .lines()
        .filter(|l| l.contains("MC-A") || l.contains("MC-B"))
        .collect();
    let row = rows
        .iter()
        .position(|l| l.contains("MC-A"))
        .ok_or_else(|| format!("MC-A is not listed:\n{list}"))?;
    let mut values: Vec<Option<&str>> = vec![None; rows.len()];
    values[row] = Some("4");
    q.guest.type_keys(&format!(
        "{}{}",
        crate::suite::install::fill(&values, &vec![1; rows.len()]),
        key::ENTER
    ))?;
    titled(&mut q.guest, "Confirm Remove of Console", deadline)?;
    q.guest.type_keys(&format!("MC-A{}", key::ENTER))?;
    q.guest.seen(&refcode::QSI_REMOVED.tag(), deadline)?;
    let closed = eventually(Duration::from_secs(10), || held.step().err())
        .ok_or("the removed console's session did not end")?;
    let Err(after) = link(&q, &a, host_print) else {
        return Err("a removed console connected again".into());
    };
    // B is still pinned.
    let mut l = link(&q, &b, host_print)?;
    l.close();
    Ok(format!(
        "no pairing out: an unpinned console refused ({unpinned}); a relayed pairing ended \
         {relay}; MC-A and MC-B paired, a third refused at Service Tools; MC-A connected pinned \
         three times; removed while connected, its session ended ({closed}) and it was refused \
         ({after}); MC-B still connects"
    ))
}
