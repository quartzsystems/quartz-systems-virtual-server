// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Each request of the host's families translated to the local request
//! of the service that owns the state, and its answer back
//! (`docs/QSI.md` section 8). A refusal carries every message the
//! service gave; a service that does not answer is `QSI0301`.

pub mod base;
pub mod convert;
pub mod host;
mod partitions;
pub mod service;
pub mod vres;

use qsi::any::{AnyReply, AnyRequest};
use qsi::host::host::msgid::SERVICE_DOWN;
use qsisess::Note;

/// The local services a request is translated to. Each is its
/// protocol's client, so a test gives fakes.
pub struct Services {
    /// The partition manager.
    pub partd: Box<dyn partd::Calls + Send>,
    /// The pool service.
    pub poold: Box<dyn poold::Calls + Send>,
    /// The switch service.
    pub vswitchd: Box<dyn vswitchd::Calls + Send>,
    /// The problem service.
    pub probd: Box<dyn problem::proto::Calls + Send>,
    /// What the host's state files say.
    pub facts: Box<dyn Facts>,
}

/// What the server reads of the host's own state files: read, never
/// written.
///
/// Each fails as a refusal: the file not there, PID 1 not answering.
pub trait Facts: Send {
    /// The host inventory PID 1 wrote at this IPL.
    fn inventory(&mut self) -> Result<state::Inventory, Vec<Note>>;
    /// Who the host is.
    fn identity(&mut self) -> Result<qsi::base::Identity, Vec<Note>>;
    /// The host's state, PID 1's.
    fn state(&mut self) -> Result<supervise::control::HostState, Vec<Note>>;
    /// Ask PID 1 to power off or restart; `by` is who asked.
    fn power(
        &mut self,
        restart: bool,
        immediate: bool,
        delay: u32,
        by: &str,
    ) -> Result<(), Vec<Note>>;
    /// The host's reference code history, oldest first.
    fn history(&mut self) -> Result<Vec<base::Posted>, Vec<Note>>;
    /// The host's clock and its display zone.
    fn time(&mut self) -> qsi::base::Time;
    /// The host's own dumps, oldest first.
    fn host_dumps(&mut self) -> Vec<hostdump::Info>;
    /// Ask PID 1 for a host dump, for `by`: its name.
    fn take_host_dump(&mut self, by: &str) -> Result<String, Vec<Note>>;
    /// Delete the host dump `name`.
    fn delete_host_dump(&mut self, name: &str) -> Result<(), Vec<Note>>;
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
    /// A host dump's bytes, host to console.
    HostDump {
        /// The dump.
        name: String,
    },
    /// An optical image's bytes, console to host.
    Upload {
        /// The image's name.
        name: String,
        /// Its size, exactly.
        bytes: u64,
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

/// Answer a request of the host's families; `user` is who the console
/// says asked, which grants nothing.
pub fn answer(s: &mut Services, request: AnyRequest, user: &str) -> Answer {
    match request {
        AnyRequest::Base(qsi::base::Request::DownloadDump(d)) => {
            Answer::Open(Open::HostDump { name: d.name })
        }
        AnyRequest::Base(r) => base::answer(s, &r, user).into(),
        AnyRequest::Partitions(r) => partitions::partitions(s, &r).into(),
        AnyRequest::Partition(r) => partitions::partition(s, r).into(),
        AnyRequest::Profiles(r) => partitions::profiles(s, r).into(),
        AnyRequest::Refcodes(r) => partitions::refcodes(s, &r).into(),
        AnyRequest::Panel(r) => partitions::panel(s, &r).into(),
        AnyRequest::Dumps(r) => partitions::dumps(s, r),
        AnyRequest::Console(r) => partitions::console(s, &r),
        AnyRequest::Host(r) => host::answer(s, &r).into(),
        AnyRequest::Vres(qsi::host::vres::Request::ImageUpload(u)) => Answer::Open(Open::Upload {
            name: u.name,
            bytes: u.bytes,
        }),
        AnyRequest::Vres(r) => vres::answer(s, r).into(),
        AnyRequest::Service(r) => service::answer(s, &r, user).into(),
        AnyRequest::Update(_) => Answer::Refused(vec![base::not_agreed()]),
        // Pairing is the connection's.
        AnyRequest::Pair(_) => Answer::Refused(vec![Note {
            msgid: qsi::msgid::UNKNOWN.into(),
            text: "This host does not answer that request.".into(),
            help: String::new(),
        }]),
    }
}

#[cfg(test)]
mod tests;
