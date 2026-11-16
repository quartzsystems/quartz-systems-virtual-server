// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager's families: partitions, partition, profiles,
//! refcodes, panel, dumps and console, each request partd's.

use qsi::Empty;
use qsi::any::AnyReply;
use qsi::host::{console, dumps, panel, partition, partitions, profiles, refcodes};
use qsisess::Note;

use super::convert::{self, no_such};
use super::{Answer, Open, Services, local, reached};

const PARTD: &str = "partition manager";

type Answered = Result<AnyReply, Vec<Note>>;

fn definition(s: &mut Services, id: u32) -> Result<model::Definition, Vec<Note>> {
    local(PARTD, s.partd.definition(id))
}

pub(super) fn partitions(s: &mut Services, r: &partitions::Request) -> Answered {
    use partitions::{Partition, Reply, Request};
    Ok(AnyReply::Partitions(match r {
        Request::List(_) => {
            let all = reached(PARTD, s.partd.list())?;
            Reply::List(all.iter().map(convert::status).collect())
        }
        Request::Get(g) => {
            let d = definition(s, g.id)?;
            let all = reached(PARTD, s.partd.list())?;
            let status = all
                .iter()
                .find(|p| p.id == g.id)
                .map(convert::status)
                .ok_or_else(|| {
                    convert::notes(&[qvsmsg::PARTITION_MISSING.with(&[&g.id.to_string()])])
                })?;
            Reply::Get(Partition {
                definition: convert::definition(&d),
                status,
            })
        }
        Request::Adapters(a) => {
            let all = reached(PARTD, s.partd.adapters(a.id))?;
            Reply::Adapters(
                all.into_iter()
                    .map(|a| partitions::Adapter {
                        partition: a.partition,
                        name: a.name,
                        profile: a.profile,
                        slot: a.slot,
                        switch: a.switch,
                        vlan: a.vlan,
                        tagged: a.tagged,
                        mac: a.mac,
                        required: a.required,
                    })
                    .collect(),
            )
        }
    }))
}

pub(super) fn partition(s: &mut Services, r: partition::Request) -> Answered {
    use partition::{Reply, Request, ShutdownKind};
    Ok(AnyReply::Partition(match r {
        Request::Create(c) => {
            let d = local(PARTD, s.partd.create(c.name))?;
            Reply::Create(convert::definition(&d))
        }
        Request::Change(c) => {
            let kept = definition(s, c.definition.id)?;
            let d = convert::definition_in(&c.definition, &kept).map_err(|n| vec![n])?;
            local(PARTD, s.partd.change(d))?;
            Reply::Change(Empty)
        }
        Request::Delete(d) => {
            local(PARTD, s.partd.delete(d.id, d.disks))?;
            Reply::Delete(Empty)
        }
        Request::Activate(a) => {
            let source = a
                .source
                .map(convert::source_in)
                .transpose()
                .map_err(|n| vec![n])?;
            let keylock = a
                .keylock
                .map(convert::keylock_in)
                .transpose()
                .map_err(|n| vec![n])?;
            let current = local(PARTD, s.partd.activate(a.id, a.profile, source, keylock))?;
            Reply::Activate(convert::current(&current))
        }
        Request::ShutDown(d) => {
            let kind = match d.kind {
                ShutdownKind::Delayed => partd::proto::Shutdown::Delayed,
                ShutdownKind::System => partd::proto::Shutdown::System,
                ShutdownKind::Immediate => partd::proto::Shutdown::Immediate,
                ShutdownKind::Other(_) => return Err(vec![no_such("shut down kind")]),
            };
            local(PARTD, s.partd.shut_down_as(d.id, kind, d.delay, d.restart))?;
            Reply::ShutDown(Empty)
        }
        Request::Restart(r) => {
            local(PARTD, s.partd.restart(r.id))?;
            Reply::Restart(Empty)
        }
    }))
}

/// A refusal led by QSI's own identifier when partd's first message is
/// one QSI names: the console acts on QSI's, and reads QS/VS's.
fn led(notes: Vec<Note>, ours: &str, theirs: &str) -> Vec<Note> {
    match notes.first() {
        Some(first) if first.msgid == theirs => {
            let mut all = vec![Note {
                msgid: ours.into(),
                text: first.text.clone(),
                help: first.help.clone(),
            }];
            all.extend(notes);
            all
        }
        _ => notes,
    }
}

/// A put's or a save's refusal, with QSI's identifiers of its conflicts.
fn saved(notes: Vec<Note>) -> Vec<Note> {
    let notes = led(notes, profiles::msgid::CHANGED, qvsmsg::PROFILE_CHANGED.id);
    let notes = led(notes, profiles::msgid::EXISTS, qvsmsg::PROFILE_EXISTS.id);
    led(
        notes,
        profiles::msgid::NOT_ACTIVE,
        qvsmsg::NOTHING_TO_SAVE.id,
    )
}

pub(super) fn profiles(s: &mut Services, r: profiles::Request) -> Answered {
    use profiles::{Entry, Reply, Request, Revised, Saved};
    Ok(AnyReply::Profiles(match r {
        Request::List(l) => {
            let all = local(PARTD, s.partd.profile_entries(l.id))?;
            Reply::List(
                all.into_iter()
                    .map(|e| Entry {
                        name: e.name,
                        revision: e.revision,
                        default: e.default,
                    })
                    .collect(),
            )
        }
        Request::Get(g) => {
            let p = local(PARTD, s.partd.profile(g.id, g.name))?;
            Reply::Get(Revised {
                revision: p.revision,
                profile: convert::profile(&p),
            })
        }
        Request::Put(p) => {
            let profile = convert::profile_in(&p.profile).map_err(|n| vec![n])?;
            let revision =
                local(PARTD, s.partd.put_profile_at(p.id, profile, p.revision)).map_err(saved)?;
            Reply::Put(Saved { revision })
        }
        Request::Delete(d) => {
            local(PARTD, s.partd.delete_profile(d.id, d.name))?;
            Reply::Delete(Empty)
        }
        Request::SetDefault(d) => {
            let mut kept = definition(s, d.id)?;
            kept.default_profile = d.name;
            local(PARTD, s.partd.change(kept))?;
            Reply::SetDefault(Empty)
        }
        Request::SaveCurrent(c) => {
            let revision =
                local(PARTD, s.partd.save_current(c.id, c.name, c.revision)).map_err(saved)?;
            Reply::SaveCurrent(Saved { revision })
        }
    }))
}

/// A page of a partition's history, the newest first.
pub(super) fn page(
    all: &[partd::Posted],
    before: Option<u64>,
    limit: u32,
) -> (Vec<refcodes::Posted>, Option<u64>) {
    let (range, more) = super::base::places(all.len(), before, limit);
    let codes = range
        .rev()
        .map(|i| convert::posted(&all[i], (i + 1) as u64))
        .collect();
    (codes, more)
}

pub(super) fn refcodes(s: &mut Services, r: &refcodes::Request) -> Answered {
    let refcodes::Request::History(h) = r;
    let all = local(PARTD, s.partd.history(h.id))?;
    let (codes, more) = page(&all, h.before, h.limit);
    Ok(AnyReply::Refcodes(refcodes::Reply::History(
        refcodes::Page { codes, more },
    )))
}

pub(super) fn panel(s: &mut Services, r: &panel::Request) -> Answered {
    let panel::Request::Function(f) = r;
    let n = |e: Note| vec![e];
    let args = partd::proto::PanelArgs {
        source: f
            .args
            .source
            .map(convert::source_in)
            .transpose()
            .map_err(n)?,
        ipl_type: f
            .args
            .ipl_type
            .map(convert::ipl_type_in)
            .transpose()
            .map_err(n)?,
        keylock: f
            .args
            .keylock
            .map(convert::keylock_in)
            .transpose()
            .map_err(n)?,
    };
    let text = local(PARTD, s.partd.panel(f.id, f.function, args))?;
    Ok(AnyReply::Panel(panel::Reply::Function(panel::Shown {
        text,
    })))
}

pub(super) fn dumps(s: &mut Services, r: dumps::Request) -> Answer {
    use dumps::{Begun, DumpInfo, Reply, Request};
    let reply = match r {
        Request::Download(d) => {
            return Answer::Open(Open::Dump {
                id: d.id,
                name: d.name,
            });
        }
        Request::List(l) => local(PARTD, s.partd.dumps(l.id)).map(|all| {
            Reply::List(
                all.into_iter()
                    .map(|d| DumpInfo {
                        name: d.name,
                        time: d.time,
                        bytes: d.bytes,
                        allocated: d.allocated,
                        code: d.code,
                        reason: d.reason,
                    })
                    .collect(),
            )
        }),
        Request::Take(t) => {
            local(PARTD, s.partd.take_dump(t.id)).map(|name| Reply::Take(Begun { name }))
        }
        Request::Delete(d) => {
            local(PARTD, s.partd.delete_dump(d.id, d.name)).map(|()| Reply::Delete(Empty))
        }
    };
    reply.map(AnyReply::Dumps).into()
}

pub(super) fn console(s: &mut Services, r: &console::Request) -> Answer {
    match r {
        console::Request::Attach(a) => Answer::Open(Open::Console {
            id: a.id,
            take_over: a.take_over,
        }),
        console::Request::Screen(c) => {
            let answered: Answered = local(PARTD, s.partd.screen(c.id)).map(|cap| {
                AnyReply::Console(console::Reply::Screen(console::Capture {
                    serial: cap.serial,
                    frame: cap.frame,
                    text: cap.text,
                }))
            });
            answered.into()
        }
    }
}
