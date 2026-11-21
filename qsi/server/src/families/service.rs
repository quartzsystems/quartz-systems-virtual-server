// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The `service` family: the host's problems, from the problem service
//! (`docs/PROBLEMS.md`). A close is the request's user's. A refusal QSI
//! names leads with QSI's identifier, then the host's message.

use qsi::any::AnyReply;
use qsi::service::{self as q, Reply, Request};
use qsisess::Note;
use supervise::control::{About, about as kind};

use super::{Services, local, reached};

fn status(s: problem::Status) -> q::ProblemStatus {
    match s {
        problem::Status::Open => q::ProblemStatus::Open,
        problem::Status::Closed => q::ProblemStatus::Closed,
    }
}

fn source(s: &problem::Source) -> q::Source {
    if s.partition == 0 {
        q::Source {
            kind: q::SourceKind::System,
            id: 0,
            name: String::new(),
        }
    } else {
        q::Source {
            kind: q::SourceKind::Partition,
            id: s.partition,
            name: s.name.clone(),
        }
    }
}

/// A problem's summary as QSI's.
#[must_use]
pub fn summary(s: &problem::Summary) -> q::Summary {
    q::Summary {
        number: s.number,
        status: status(s.status),
        first_seen: s.first_seen,
        last_seen: s.last_seen,
        count: s.count,
        refcode: qsi::base::Refcode {
            seq: s.refcode.seq,
            code: s.refcode.code.clone(),
            words: s.refcode.words.clone(),
            text: s.refcode.text.clone(),
            time_ms: s.refcode.time_ms,
        },
        source: source(&s.source),
        description: s.description.clone(),
        msgid: s.msgid.clone(),
        holds: s.holds,
        closed_by: s.closed_by.clone(),
        closed_at: s.closed_at,
    }
}

/// A resource as QSI's: a service has no kind of its own, and is the
/// system's, named as a service.
#[must_use]
pub fn resource(a: &About) -> q::Resource {
    let (kind, name) = match a.kind.as_str() {
        kind::DISK_UNIT => (q::ResourceKind::DiskUnit, a.name.clone()),
        kind::STORAGE_POOL => (q::ResourceKind::StoragePool, a.name.clone()),
        kind::VIRTUAL_DISK => (q::ResourceKind::VirtualDisk, a.name.clone()),
        kind::NETWORK_INTERFACE => (q::ResourceKind::NetworkInterface, a.name.clone()),
        kind::VIRTUAL_SWITCH => (q::ResourceKind::VirtualSwitch, a.name.clone()),
        kind::PARTITION => (q::ResourceKind::Partition, a.name.clone()),
        kind::CODE_SLOT => (q::ResourceKind::CodeSlot, a.name.clone()),
        kind::SERVICE => (q::ResourceKind::System, format!("service {}", a.name)),
        _ => (q::ResourceKind::System, a.name.clone()),
    };
    q::Resource {
        kind,
        name,
        location: a.location.clone(),
        serial: a.serial.clone(),
    }
}

fn problem(p: &problem::Problem) -> q::Problem {
    q::Problem {
        summary: summary(&p.summary),
        resources: p.resources.iter().map(resource).collect(),
        log: p
            .log
            .iter()
            .map(|l| q::Line {
                time: l.time,
                text: l.text.clone(),
            })
            .collect(),
    }
}

/// QSI's filter as the problem service's: a source by its kind and ID.
fn filter(f: &q::Filter) -> problem::Filter {
    problem::Filter {
        status: f.status.and_then(|s| match s {
            q::ProblemStatus::Open => Some(problem::Status::Open),
            q::ProblemStatus::Closed => Some(problem::Status::Closed),
            q::ProblemStatus::Other(_) => None,
        }),
        partition: f.source.as_ref().map(|s| match s.kind {
            q::SourceKind::Partition => s.id,
            _ => 0,
        }),
        since: f.since,
    }
}

/// A refusal of the problem service led by QSI's identifier for it.
fn led(notes: Vec<Note>) -> Vec<Note> {
    let ours = match notes.first().map(|n| n.msgid.as_str()) {
        Some("QVS5001") => q::msgid::NO_PROBLEM,
        Some("QVS5002") => q::msgid::CLOSED,
        Some("QVS5003") => q::msgid::STILL_HOLDS,
        _ => return notes,
    };
    let first = &notes[0];
    let mut all = vec![Note {
        msgid: ours.into(),
        text: first.text.clone(),
        help: first.help.clone(),
    }];
    all.extend(notes);
    all
}

/// Answer a request of the `service` family; a close is `user`'s.
pub(super) fn answer(s: &mut Services, r: &Request, user: &str) -> Result<AnyReply, Vec<Note>> {
    const NAME: &str = "problem service";
    Ok(AnyReply::Service(match r {
        Request::List(l) => {
            let page = reached(NAME, s.probd.list(filter(&l.filter), l.before, l.limit))?;
            Reply::List(q::Page {
                problems: page.problems.iter().map(summary).collect(),
                more: page.more,
            })
        }
        Request::Get(g) => Reply::Get(problem(&local(NAME, s.probd.get(g.number)).map_err(led)?)),
        Request::Close(c) => {
            let closed =
                local(NAME, s.probd.close(c.number, c.anyway, user.to_string())).map_err(led)?;
            Reply::Close(summary(&closed.summary))
        }
        Request::Note(_) | Request::ListRated(_) | Request::GetRated(_) => {
            return Err(vec![super::base::not_agreed()]);
        }
    }))
}

/// A problem service's event as QSI's.
#[must_use]
pub fn event(e: &problem::Noticed) -> qsi::any::AnyEvent {
    let n = q::Noticed {
        problem: summary(&e.problem),
        time: e.time,
    };
    qsi::any::AnyEvent::Service(match e.notice {
        problem::Notice::Opened => q::Event::Opened(n),
        problem::Notice::Seen => q::Event::Seen(n),
        problem::Notice::Cleared => q::Event::Cleared(n),
        problem::Notice::Closed => q::Event::Closed(n),
    })
}
