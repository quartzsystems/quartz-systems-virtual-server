// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The base family's requests that are the host's: who it is, its
//! state, power, its reference code history and its clock. Events are
//! the connection's.

use qsi::Empty;
use qsi::any::AnyReply;
use qsi::base::{
    DumpBegun, DumpKind, PowerAction, PowerState, Refcode, RefcodeList, Reply, Request, State,
    SystemDump,
};
use qsisess::Note;

use super::Services;
use super::convert::no_such;

/// The most codes on one page.
const MAX_PAGE: u32 = 256;

/// One code of the host's history, as the log service kept it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Posted {
    /// The code.
    pub code: String,
    /// Its text.
    pub text: String,
    /// When, in Unix milliseconds.
    pub time_ms: u64,
}

/// The indexes of a page of a history of `len` codes kept oldest
/// first: places count from 1, the page is the `limit` before place
/// `before` (the newest when none), and the next page's `before` is
/// its oldest place, when there are older.
pub fn places(
    len: usize,
    before: Option<u64>,
    limit: u32,
) -> (std::ops::Range<usize>, Option<u64>) {
    let limit = usize::try_from(limit.clamp(1, MAX_PAGE)).unwrap_or(1);
    let end = before
        .and_then(|b| usize::try_from(b.saturating_sub(1)).ok())
        .map_or(len, |b| b.min(len));
    let start = end.saturating_sub(limit);
    (start..end, (start > 0).then_some((start + 1) as u64))
}

/// A page of the host's history, the newest first.
pub fn page(all: &[Posted], before: Option<u64>, limit: u32) -> RefcodeList {
    let (range, more) = places(all.len(), before, limit);
    let codes = range
        .rev()
        .map(|i| Refcode {
            seq: (i + 1) as u64,
            code: all[i].code.clone(),
            words: Vec::new(),
            text: all[i].text.clone(),
            time_ms: all[i].time_ms,
        })
        .collect();
    RefcodeList { codes, more }
}

/// The host's state as QSI says it.
pub fn state(s: &supervise::control::HostState) -> State {
    use supervise::control::Power;
    State {
        power: match s.power {
            Power::Starting => PowerState::Starting,
            Power::Running => PowerState::Running,
            Power::Ending => PowerState::Ending,
        },
        refcode: (!s.code.is_empty()).then(|| Refcode {
            seq: 0,
            code: s.code.clone(),
            words: Vec::new(),
            text: s.text.clone(),
            time_ms: 0,
        }),
        release: s.release.clone(),
        slot: s.slot.clone(),
        keylock: None,
    }
}

/// A host dump as QSI's.
#[must_use]
pub fn system_dump(i: &hostdump::Info) -> SystemDump {
    SystemDump {
        name: i.name.clone(),
        kind: DumpKind::ServiceData,
        kind_name: hostdump::KIND_NAME.into(),
        bytes: i.bytes,
        time: i.time,
        by: i.by.clone(),
    }
}

fn no_kind() -> Note {
    Note {
        msgid: qsi::base::msgid::NO_KIND.into(),
        text: "This host takes dumps of its service data only.".into(),
        help: "A host dump holds the logs, the histories, the state without its secrets and the services' status (docs/DUMPS.md).".into(),
    }
}

/// No host dump of that name.
#[must_use]
pub fn no_dump(name: &str) -> Note {
    Note {
        msgid: qsi::base::msgid::NO_DUMP.into(),
        text: format!("There is no host dump {name}."),
        help: String::new(),
    }
}

/// A host dump being taken already.
#[must_use]
pub fn taking(why: &str) -> Note {
    Note {
        msgid: qsi::base::msgid::TAKING.into(),
        text: why.into(),
        help: String::new(),
    }
}

/// A request of a group this server's hello did not say.
pub(super) fn not_agreed() -> Note {
    Note {
        msgid: qsi::msgid::NOT_AGREED.into(),
        text: "This host does not serve that family.".into(),
        help: String::new(),
    }
}

/// Answer a base request that is the host's; `user` is who the
/// console says asked, for the power down's record.
pub(super) fn answer(s: &mut Services, r: &Request, user: &str) -> Result<AnyReply, Vec<Note>> {
    Ok(AnyReply::Base(match r {
        Request::Identify(_) => Reply::Identify(s.facts.identity()?),
        Request::State(_) => Reply::State(state(&s.facts.state()?)),
        Request::Power(p) => {
            let restart = match p.action {
                PowerAction::Off => false,
                PowerAction::Restart => true,
                PowerAction::Other(_) => return Err(vec![no_such("power action")]),
            };
            s.facts.power(restart, p.immediate, p.delay, user)?;
            Reply::Power(Empty)
        }
        Request::Refcodes(h) => Reply::Refcodes(page(&s.facts.history()?, h.before, h.limit)),
        Request::Time(_) => Reply::Time(s.facts.time()),
        Request::Dumps(_) => Reply::Dumps(s.facts.host_dumps().iter().map(system_dump).collect()),
        Request::TakeDump(t) => match t.kind {
            DumpKind::ServiceData => {
                let by = format!("a management console, user {user:?}");
                Reply::TakeDump(DumpBegun {
                    name: s.facts.take_host_dump(&by)?,
                })
            }
            _ => return Err(vec![no_kind()]),
        },
        Request::DeleteDump(d) => {
            s.facts.delete_host_dump(&d.name)?;
            Reply::DeleteDump(Empty)
        }
        Request::DumpSpace(_) => {
            let all = s.facts.host_dumps();
            Reply::DumpSpace(qsi::base::Space {
                bytes: all.iter().map(|d| d.bytes).sum(),
                limit: hostdump::store::MAX_TOTAL,
                dumps: u32::try_from(all.len()).unwrap_or(u32::MAX),
                max_dumps: u32::try_from(hostdump::store::MAX_DUMPS).unwrap_or(u32::MAX),
            })
        }
        // The connection's: it opens a stream; or a bit this host does
        // not say (it keeps no fix level of its own).
        Request::DownloadDump(_) | Request::FixLevel(_) => {
            return Err(vec![not_agreed()]);
        }
        // The connection's: they need the event log.
        Request::Subscribe(_) | Request::Unsubscribe(_) => {
            return Err(vec![Note {
                msgid: qsi::msgid::UNKNOWN.into(),
                text: "Events are the connection's.".into(),
                help: String::new(),
            }]);
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hosts_history_is_paged_by_place() {
        let all: Vec<Posted> = (1..=3)
            .map(|n| Posted {
                code: format!("C200100{n}"),
                text: String::new(),
                time_ms: n,
            })
            .collect();
        let p = page(&all, None, 2);
        let places: Vec<u64> = p.codes.iter().map(|c| c.seq).collect();
        assert_eq!((places, p.more), (vec![3, 2], Some(2)));
        let p = page(&all, p.more, 2);
        assert_eq!((p.codes.len(), p.more), (1, None));
        assert_eq!(page(&all, Some(99), 1).codes[0].seq, 3);
    }
}
