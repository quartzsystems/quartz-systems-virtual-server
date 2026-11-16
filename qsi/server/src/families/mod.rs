// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Each request of the host's families translated to the local request
//! of the service that owns the state, and its answer back
//! (`docs/QSI.md` section 8). A refusal carries every message the
//! service gave; a service that does not answer is `QSI0301`.

pub mod convert;
mod partitions;

use qsi::any::{AnyReply, AnyRequest};
use qsi::host::host::msgid::SERVICE_DOWN;
use qsisess::Note;

/// The local services a request is translated to. Each is its
/// protocol's client, so a test gives fakes.
pub struct Services {
    /// The partition manager.
    pub partd: Box<dyn partd::Calls + Send>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services").finish_non_exhaustive()
    }
}

/// What a request opens beside its reply: the connection serves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// A partition's console, taken over or not.
    Console {
        /// The partition.
        id: u32,
        /// Whether another viewer is put off.
        take_over: bool,
    },
    /// A dump's bytes, host to console.
    Dump {
        /// The partition.
        id: u32,
        /// The dump.
        name: String,
    },
}

/// How a request is answered.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // an answer is passed once
pub enum Answer {
    /// A reply.
    Reply(AnyReply),
    /// Refused, with every message that applies, the first first.
    Refused(Vec<Note>),
    /// A stream to open; the connection replies once it is open.
    Open(Open),
}

impl From<Result<AnyReply, Vec<Note>>> for Answer {
    fn from(r: Result<AnyReply, Vec<Note>>) -> Self {
        match r {
            Ok(reply) => Answer::Reply(reply),
            Err(notes) => Answer::Refused(notes),
        }
    }
}

/// The local service `name` did not answer.
pub fn down(name: &str, e: &ipc::WireError) -> Vec<Note> {
    vec![Note {
        msgid: SERVICE_DOWN.into(),
        text: format!("The host's {name} did not answer."),
        help: format!("Its socket: {e}."),
    }]
}

/// A local call's two failures, as notes: the service down, or its
/// refusal.
pub(crate) fn local<T>(
    name: &str,
    r: Result<Result<T, Vec<qvsmsg::Issued>>, ipc::WireError>,
) -> Result<T, Vec<Note>> {
    r.map_err(|e| down(name, &e))?
        .map_err(|issued| convert::notes(&issued))
}

/// A local call that does not refuse.
pub(crate) fn reached<T>(name: &str, r: Result<T, ipc::WireError>) -> Result<T, Vec<Note>> {
    r.map_err(|e| down(name, &e))
}

/// Answer a request of the host's families.
pub fn answer(s: &mut Services, request: AnyRequest) -> Answer {
    match request {
        AnyRequest::Partitions(r) => partitions::partitions(s, &r).into(),
        AnyRequest::Partition(r) => partitions::partition(s, r).into(),
        AnyRequest::Profiles(r) => partitions::profiles(s, r).into(),
        AnyRequest::Refcodes(r) => partitions::refcodes(s, &r).into(),
        AnyRequest::Panel(r) => partitions::panel(s, &r).into(),
        AnyRequest::Dumps(r) => partitions::dumps(s, r),
        AnyRequest::Console(r) => partitions::console(s, &r),
        // Base and pairing are the connection's; the rest are not yet.
        _ => Answer::Refused(vec![Note {
            msgid: qsi::msgid::UNKNOWN.into(),
            text: "This host does not answer that request.".into(),
            help: String::new(),
        }]),
    }
}

#[cfg(test)]
mod tests;
