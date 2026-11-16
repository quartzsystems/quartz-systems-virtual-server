// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The `update` family: code update, from the update service
//! (`docs/UPDATE.md`). A stage opens a stream to it; an activation is
//! the request's user's. A refusal QSI names leads with QSI's
//! identifier (`QSI1101` to `QSI1114`, the host's `QVS5301` to
//! `QVS5314`), then the host's message.

use qsi::any::{AnyEvent, AnyReply};
use qsi::update::{self as q, Reply, Request};
use qsisess::Note;

use super::{Answer, Open, Services, convert, local, reached};

const NAME: &str = "update service";

fn attempt(a: update::proto::Attempt) -> q::Attempt {
    match a {
        update::proto::Attempt::Never => q::Attempt::Never,
        update::proto::Attempt::Pending => q::Attempt::Pending,
        update::proto::Attempt::Confirmed => q::Attempt::Confirmed,
        update::proto::Attempt::RolledBack => q::Attempt::RolledBack,
    }
}

fn verification(v: update::proto::Verification) -> q::Verification {
    match v {
        update::proto::Verification::Receiving => q::Verification::Receiving,
        update::proto::Verification::Verifying => q::Verification::Verifying,
        update::proto::Verification::Verified => q::Verification::Verified,
        update::proto::Verification::Refused => q::Verification::Refused,
    }
}

/// An image's description as QSI's.
#[must_use]
pub fn image(i: &update::proto::ImageInfo) -> q::Image {
    q::Image {
        product: i.product.clone(),
        release: i.release.clone(),
        build: i.build.clone(),
        fixes: i.fixes.clone(),
        bytes: i.bytes,
    }
}

fn first(issued: &[qvsmsg::Issued]) -> Option<qsi::host::model::Note> {
    led(convert::notes(issued))
        .first()
        .map(|n| qsi::host::model::Note {
            msgid: n.msgid.clone(),
            text: n.text.clone(),
            help: n.help.clone(),
        })
}

fn status(s: &update::proto::Status) -> q::Status {
    q::Status {
        slots: s
            .slots
            .iter()
            .map(|s| q::Slot {
                name: s.name.clone(),
                release: s.release.clone(),
                build: s.build.clone(),
                fixes: s.fixes.clone(),
                running: s.running,
                default: s.default,
                last: attempt(s.last),
                reason: s.reason.clone(),
                time: s.time,
            })
            .collect(),
        staged: s.staged.as_ref().map(|t| q::StagedImage {
            name: t.name.clone(),
            bytes: t.bytes,
            verification: verification(t.verification),
            image: t.image.as_ref().map(image),
            error: first(&t.error),
            time: t.time,
        }),
        installing: s.installing.as_ref().map(progress),
    }
}

fn progress(p: &update::proto::Progress) -> q::Progress {
    q::Progress {
        slot: p.slot.clone(),
        percent: p.percent,
        time: p.time,
    }
}

/// A refusal of the update service, led by QSI's identifier for it:
/// `QVS53nn` is `QSI11nn` for 01 to 14.
#[must_use]
pub fn led(notes: Vec<Note>) -> Vec<Note> {
    let ours = notes.first().and_then(|n| {
        let nn: u32 = n.msgid.strip_prefix("QVS53")?.parse().ok()?;
        (1..=14).contains(&nn).then(|| format!("QSI11{nn:02}"))
    });
    let Some(ours) = ours else { return notes };
    let mut all = vec![Note {
        msgid: ours,
        text: notes[0].text.clone(),
        help: notes[0].help.clone(),
    }];
    all.extend(notes);
    all
}

/// Answer a request of the `update` family; an activation is `user`'s.
pub(super) fn answer(s: &mut Services, r: Request, user: &str) -> Answer {
    let reply = match r {
        Request::Status(_) => reached(NAME, s.updd.status()).map(|st| Reply::Status(status(&st))),
        Request::Stage(st) => {
            return Answer::Open(Open::Stage {
                name: st.name,
                bytes: st.bytes,
                allow_older: st.allow_older,
            });
        }
        Request::Discard(_) => local(NAME, s.updd.discard()).map(|()| Reply::Discard(qsi::Empty)),
        Request::Install(_) => {
            local(NAME, s.updd.install()).map(|slot| Reply::Install(q::Target { slot }))
        }
        Request::Activate(a) => {
            let shut_down = match a.partitions {
                q::Partitions::ShutDown => true,
                q::Partitions::Refuse => false,
                q::Partitions::Other(_) => {
                    return Answer::Refused(vec![convert::no_such("way of treating partitions")]);
                }
            };
            let by = format!("a management console, user {user:?}");
            local(NAME, s.updd.activate(shut_down, a.timeout, by))
                .map(|()| Reply::Activate(qsi::Empty))
        }
        Request::Cancel(_) => local(NAME, s.updd.cancel()).map(|()| Reply::Cancel(qsi::Empty)),
    };
    match reply {
        Ok(r) => Answer::Reply(AnyReply::Update(r)),
        Err(notes) => Answer::Refused(led(notes)),
    }
}

/// An update service's event as QSI's.
#[must_use]
pub fn event(e: &update::proto::Event) -> AnyEvent {
    use update::proto::Event as E;
    AnyEvent::Update(match e {
        E::Staged {
            name,
            bytes,
            error,
            time,
        } => q::Event::Staged(q::Staged {
            name: name.clone(),
            bytes: *bytes,
            error: first(error),
            time: *time,
        }),
        E::Verified {
            name,
            image: i,
            error,
            time,
        } => q::Event::Verified(q::Verified {
            name: name.clone(),
            image: i.as_ref().map(image),
            error: first(error),
            time: *time,
        }),
        E::Installing { progress: p } => q::Event::Installing(progress(p)),
        E::Installed {
            slot,
            image: i,
            error,
            time,
        } => q::Event::Installed(q::Installed {
            slot: slot.clone(),
            image: i.as_ref().map(image),
            error: first(error),
            time: *time,
        }),
        E::Activating { slot, time } => q::Event::Activating(q::Activating {
            slot: slot.clone(),
            time: *time,
        }),
    })
}

/// The events PID 1's codes say after the restart: a slot confirmed
/// (`C2012060`, `<release> build <build> in slot <X>`) or rolled back
/// (`A2012070`, `slot <X> did not confirm; ...`).
#[must_use]
pub fn from_code(code: &str, text: &str, time: u64) -> Option<AnyEvent> {
    let detail = text.split_once(": ").map_or("", |(_, d)| d);
    if code == refcode::UPDATE_CONFIRMED.code {
        let mut w = detail.split(' ');
        let release = w.next()?.to_string();
        let build = (w.next() == Some("build"))
            .then(|| w.next())
            .flatten()?
            .to_string();
        let slot = detail.rsplit(' ').next()?.to_string();
        return Some(AnyEvent::Update(q::Event::Confirmed(q::Confirmed {
            slot,
            release,
            build,
            time,
        })));
    }
    if code == refcode::UPDATE_ROLLED_BACK.code {
        let slot = detail.strip_prefix("slot ")?.split(' ').next()?.to_string();
        return Some(AnyEvent::Update(q::Event::RolledBack(q::RolledBack {
            slot,
            reason: detail.to_string(),
            code: refcode::SLOT_FELL_BACK.code.to_string(),
            time,
        })));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosts_refusals_are_led_by_qsis() {
        let notes = convert::notes(&[qvsmsg::UPDATE_OLDER.with(&["X", "V1R0M0", "V1R0M1"])]);
        let led = led(notes);
        assert_eq!(
            (led[0].msgid.as_str(), led[1].msgid.as_str()),
            ("QSI1104", "QVS5304")
        );
        let other = convert::notes(&[qvsmsg::UPDATE_NOT_WRITTEN.with(&["B", "x"])]);
        assert_eq!(super::led(other)[0].msgid, "QVS5315");
    }

    #[test]
    fn the_restarts_codes_are_its_events() {
        let c = from_code(
            "C2012060",
            "Update confirmed: V1R0M1 build phase9 in slot B",
            7,
        );
        let Some(AnyEvent::Update(q::Event::Confirmed(c))) = c else {
            panic!()
        };
        assert_eq!(
            (c.slot.as_str(), c.release.as_str(), c.build.as_str()),
            ("B", "V1R0M1", "phase9")
        );
        let r = from_code(
            "A2012070",
            "Update rolled back: slot B did not confirm; this IPL is on slot A",
            8,
        );
        let Some(AnyEvent::Update(q::Event::RolledBack(r))) = r else {
            panic!()
        };
        assert_eq!((r.slot.as_str(), r.code.as_str()), ("B", "A2001035"));
        assert!(from_code("C2001000", "Host IPL started", 1).is_none());
    }
}
