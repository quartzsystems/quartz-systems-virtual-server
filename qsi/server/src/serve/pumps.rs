// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What feeds the event log, each a thread: the partition manager's and
//! the switch service's subscriptions, a look at the pool service every
//! two seconds, the host's history and PID 1's state, and what the
//! trust says happened. A source that goes is subscribed to again, and
//! each source taken up is compared with what the log last said, so
//! nothing it said while the server was away is missed.

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use partd::Calls as _;
use poold::Calls as _;
use qsi::any::AnyEvent;
use qsi::base;

use crate::events::hub::{self, Shared};
use crate::events::said;
use crate::events::translate::{self, Look};
use crate::families::Facts;
use crate::families::base::state as base_state;
use crate::families::host::resources;
use crate::trust::{Happened, Trust};

use super::facts::HostFacts;

/// How long a pump waits before it tries a source again.
const AGAIN: Duration = Duration::from_secs(2);
/// How often the pool service and the host are looked at.
const LOOK: Duration = Duration::from_secs(2);

fn now_secs() -> u64 {
    u64::try_from(appliance::time::now_secs()).unwrap_or(0)
}

fn push(hub: &Shared, e: &AnyEvent) {
    // One too long for a message is not kept; none of these is.
    let _ = hub::lock(hub).push(e);
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = thread::Builder::new().name(name.into()).spawn(f);
}

/// The partition manager's events, and the host's resources after each
/// that moves them.
pub fn partd(socket: PathBuf, hub: Shared) {
    spawn("partd-events", move || {
        loop {
            let client = partd::Client::new(&socket);
            if let Ok(mut events) = client.events() {
                let mut c = client.clone();
                if let Ok(list) = c.list() {
                    let missed = {
                        let h = hub::lock(&hub);
                        let mut c = client.clone();
                        said::partitions(
                            h.said(),
                            &list,
                            |id| c.history(id).ok().and_then(Result::ok).unwrap_or_default(),
                            now_secs(),
                        )
                    };
                    hub::lock(&hub).take_up("partd", &missed);
                }
                while let Ok(e) = events.next(None) {
                    push(&hub, &translate::partd(&e));
                    if translate::moves_resources(&e) {
                        let mut c = client.clone();
                        if let (Ok(r), Ok(s), Ok(all)) = (c.resources(), c.settings(), c.list()) {
                            let host = resources(&r, &s, u32::try_from(all.len()).unwrap_or(0));
                            push(
                                &hub,
                                &AnyEvent::Host(qsi::host::host::Event::Resources(host)),
                            );
                        }
                    }
                }
            }
            thread::sleep(AGAIN);
        }
    });
}

/// The switch service's events.
pub fn vswitchd(socket: PathBuf, hub: Shared) {
    spawn("vswitchd-events", move || {
        loop {
            let client = vswitchd::Client::new(&socket);
            let ready = || {
                let mut c = client.clone();
                if let Ok(list) = vswitchd::Calls::switches(&mut c) {
                    let mut h = hub::lock(&hub);
                    let missed = said::switches(h.said(), &list, now_secs());
                    h.take_up("vswitchd", &missed);
                }
            };
            let _ = client.subscribed(ready, |e| {
                push(&hub, &translate::vswitchd(&e));
                true
            });
            thread::sleep(AGAIN);
        }
    });
}

/// The problem service's events; on taking it up, each problem compared
/// with what the log last said of it.
pub fn probd(socket: PathBuf, hub: Shared) {
    spawn("probd-events", move || {
        loop {
            let client = problem::proto::Client::new(&socket);
            let ready = || {
                let mut all = Vec::new();
                let mut before = None;
                let mut c = client.clone();
                // Every problem kept, a page at a time.
                while let Ok(page) =
                    problem::proto::Calls::list(&mut c, problem::Filter::default(), before, 500)
                {
                    all.extend(page.problems);
                    match page.more {
                        Some(m) => before = Some(m),
                        None => break,
                    }
                }
                let mut h = hub::lock(&hub);
                let missed = said::problems(h.said(), &all, now_secs());
                h.take_up("probd", &missed);
            };
            let _ = client.events(ready, |e| {
                push(&hub, &crate::families::service::event(&e));
                true
            });
            thread::sleep(AGAIN);
        }
    });
}

/// The pool service, which sends no events: what changed between two
/// looks.
pub fn poold(socket: PathBuf, hub: Shared) {
    spawn("pool-looks", move || {
        let mut client = poold::Client::new(&socket);
        // The first look is compared with what the log last said.
        let mut before: Option<Look> = None;
        loop {
            let look = (|| {
                Some(Look::of(
                    &client.pools().ok()?,
                    &client.vdisks(String::new()).ok()?,
                    &client.images().ok()?,
                    &client.volumes().ok()?,
                ))
            })();
            if let Some(look) = look {
                if let Some(b) = &before {
                    for e in translate::pools(b, &look, now_secs()) {
                        push(&hub, &e);
                    }
                } else {
                    let mut h = hub::lock(&hub);
                    let missed = translate::pools(&h.said().look(), &look, now_secs());
                    h.take_up("poold", &missed);
                }
                before = Some(look);
            }
            thread::sleep(LOOK);
        }
    });
}

/// The host: each code its history gains, each change of PID 1's
/// state.
pub fn host(mut facts: HostFacts, hub: Shared) {
    spawn("host-looks", move || {
        // From the last code the log said; the whole history is the
        // first look's when it never said one.
        let said = hub::lock(&hub).said().host_seen();
        let mut seen = said.map_or_else(
            || facts.history().map_or(0, |h| h.len()),
            |s| usize::try_from(s).unwrap_or(usize::MAX),
        );
        let mut state = None;
        loop {
            if let Ok(all) = facts.history() {
                for (i, p) in all.iter().enumerate().skip(seen) {
                    push(
                        &hub,
                        &AnyEvent::Base(base::Event::Refcode(base::Refcode {
                            seq: (i + 1) as u64,
                            code: p.code.clone(),
                            words: Vec::new(),
                            text: p.text.clone(),
                            time_ms: p.time_ms,
                        })),
                    );
                }
                seen = all.len();
            }
            if let Ok(now) = facts.state() {
                let now = base_state(&now);
                if state.as_ref() != Some(&now) {
                    if state.is_some() {
                        push(&hub, &AnyEvent::Base(base::Event::State(now.clone())));
                    }
                    state = Some(now);
                }
            }
            thread::sleep(LOOK);
        }
    });
}

/// What the trust says happened: a reference code each, through PID 1
/// (which the host's history and so a base event follow).
pub fn trust(trust: Trust, control: PathBuf, say: impl Fn(&str) + Send + 'static) {
    spawn("trust", move || {
        loop {
            trust.expire();
            for h in trust.happened() {
                let (code, detail) = match &h {
                    Happened::Paired(c) => (
                        &refcode::QSI_PAIRED,
                        format!("{}, {}", c.name, &c.fingerprint[..16]),
                    ),
                    Happened::Failed(left) => (
                        &refcode::QSI_PAIRING_FAILED,
                        format!("{left} attempts left"),
                    ),
                    Happened::Expired => (&refcode::QSI_PAIRING_ENDED, String::new()),
                    Happened::Removed(c) => (
                        &refcode::QSI_REMOVED,
                        format!("{}, {}", c.name, &c.fingerprint[..16]),
                    ),
                };
                let text = code.text_with(&detail);
                say(&format!("{} {text}", code.code));
                let mut c = supervise::control::Client::new(&control);
                let _ = supervise::control::Calls::post(&mut c, code.code.to_string(), text);
                // A password's last attempt failed: failing again and again.
                if matches!(h, Happened::Failed(0)) {
                    let code = &refcode::QSI_PAIRING_FAILING;
                    let text = code.text_with("a pairing password's attempts were all spent");
                    say(&format!("{} {text}", code.code));
                    let _ = supervise::control::Calls::post(&mut c, code.code.to_string(), text);
                }
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}
