// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager's socket (`docs/PARTD.md`): one request and
//! one answer per connection, each an `ipc` frame, as PID 1's control
//! socket. A request that fails answers with `QVS` messages. One
//! request is different: after `subscribe` is answered the connection
//! stays open and every event follows on it, a frame each.

use model::{Definition, IplSource, Keylock, Profile};
use qvsmsg::Issued;
use resource::{Current, HostResources, Pool, Settings};

/// The service's name in the service table.
pub const NAME: &str = "partd";
/// The socket's file name in the run directory.
pub const SOCKET_FILE: &str = "qsvspartd.sock";
/// The partition manager's own directory in the run directory.
pub const RUN: &str = "partd";

/// The report of the partition `id`'s VMM under the run directory
/// `run_dir`: what the VMM wrote on its standard error, a line each.
pub fn report(run_dir: &std::path::Path, id: u32) -> std::path::PathBuf {
    run_dir.join(RUN).join(format!("{id}.report"))
}

/// What a partition is doing, by the names QS/MC shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// It has no VMM and holds nothing.
    NotActivated,
    /// Its VMM runs and has not yet entered the guest's kernel.
    Starting,
    /// The guest runs.
    Running,
    /// It was asked to end and has not yet.
    ShuttingDown,
    /// Its VMM failed or was killed. It holds nothing, and can be
    /// activated again.
    Error,
}
ipc::wire_enum!(State {
    0 => NotActivated,
    1 => Starting,
    2 => Running,
    3 => ShuttingDown,
    4 => Error,
});

impl State {
    /// As it is shown.
    pub fn word(self) -> &'static str {
        match self {
            State::NotActivated => "Not activated",
            State::Starting => "Starting",
            State::Running => "Running",
            State::ShuttingDown => "Shutting down",
            State::Error => "Error",
        }
    }

    /// Whether the partition has a VMM and holds resources.
    pub fn active(self) -> bool {
        matches!(self, State::Starting | State::Running | State::ShuttingDown)
    }
}

/// One partition, as a list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// Its ID.
    pub id: u32,
    /// Its name.
    pub name: String,
    /// Its state.
    pub state: State,
    /// Its current reference code; empty when it has posted none.
    pub code: String,
    /// The code's text.
    pub text: String,
    /// What it holds; `None` when it is not activated.
    pub current: Option<Current>,
    /// The profile it was last activated with.
    pub profile: String,
    /// How its VMM last ended: `exit <code>` or `signal <number>`;
    /// empty when it has not.
    pub last_end: String,
    /// Why it is not activated or in error, when a request did not
    /// say: the message of an activation that failed after it was
    /// answered.
    pub message: Option<Issued>,
    /// Its VMM's process; 0 when it has none.
    pub pid: u32,
}
ipc::wire_struct!(Status {
    id,
    name,
    state,
    code,
    text,
    current,
    profile,
    last_end,
    message,
    pid
});

/// One reference code of a partition's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posted {
    /// When, in seconds since the epoch.
    pub time: u64,
    /// The code.
    pub code: String,
    /// Its text.
    pub text: String,
}
ipc::wire_struct!(Posted { time, code, text });

/// Something that happened to a partition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Its state changed.
    State {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// The state it is in now.
        state: State,
        /// When.
        time: u64,
    },
    /// It posted a reference code.
    Code {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// The code.
        posted: Posted,
    },
    /// What it holds changed: resolved at an activation, given back at
    /// an end.
    Resources {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// What it holds now.
        current: Option<Current>,
        /// When.
        time: u64,
    },
    /// It was created, changed or deleted.
    Defined {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// Whether its definition still exists.
        exists: bool,
        /// When.
        time: u64,
    },
}
ipc::wire_enum!(Event {
    0 => State { id, name, state, time },
    1 => Code { id, name, posted },
    2 => Resources { id, name, current, time },
    3 => Defined { id, name, exists, time },
});

impl Event {
    /// The partition it is about.
    pub fn partition(&self) -> u32 {
        match self {
            Event::State { id, .. }
            | Event::Code { id, .. }
            | Event::Resources { id, .. }
            | Event::Defined { id, .. } => *id,
        }
    }
}

/// What a request that failed answers with: every message.
pub type Refused = Vec<Issued>;

ipc::rpc! {
    request Request, service Partitions, client Calls, dispatch dispatch;
    /// Every partition, in ID order, with its state, its reference
    /// code and what it holds.
    0 => fn list / List () -> Vec<Status>;
    /// Create a partition called `name`. Its ID and serial number are
    /// assigned; everything else has its default.
    1 => fn create / Create (name: String) -> Result<Definition, Refused>;
    /// A partition's definition.
    2 => fn definition / GetDefinition (id: u32) -> Result<Definition, Refused>;
    /// Change a definition: its name, keylock, default profile and
    /// auto start. Only while the partition is not activated.
    3 => fn change / Change (definition: Definition) -> Result<(), Refused>;
    /// Delete a partition and its profiles. Only while it is not
    /// activated. Its backing stores are left alone.
    4 => fn delete / Delete (id: u32) -> Result<(), Refused>;
    /// The names of a partition's profiles.
    5 => fn profiles / ListProfiles (id: u32) -> Result<Vec<String>, Refused>;
    /// One profile.
    6 => fn profile / GetProfile (id: u32, name: String) -> Result<Profile, Refused>;
    /// Create a profile or replace the one of its name. The first
    /// profile of a partition becomes its default.
    7 => fn put_profile / PutProfile (id: u32, profile: Profile) -> Result<(), Refused>;
    /// Delete a profile; not the default one.
    8 => fn delete_profile / DeleteProfile (id: u32, name: String) -> Result<(), Refused>;
    /// Activate a partition with a profile (the default one when the
    /// name is empty), with an IPL source or a keylock position for
    /// this activation only when one is given. What it was given.
    9 => fn activate / Activate (id: u32, profile: String, source: Option<IplSource>, keylock: Option<Keylock>) -> Result<Current, Refused>;
    /// Shut a partition down at once: its VMM is stopped.
    10 => fn shut_down / ShutDown (id: u32) -> Result<(), Refused>;
    /// Shut a partition down at once and activate it again with the
    /// profile it had.
    11 => fn restart / Restart (id: u32) -> Result<(), Refused>;
    /// The host's resources: installed, configurable, available, and
    /// each pool.
    12 => fn resources / Resources () -> HostResources;
    /// Create a shared processor pool.
    13 => fn create_pool / CreatePool (pool: Pool) -> Result<(), Refused>;
    /// Change a pool's cores or its maximum. Only while no partition
    /// runs in it.
    14 => fn change_pool / ChangePool (pool: Pool) -> Result<(), Refused>;
    /// Delete a pool. Only while no partition runs in it.
    15 => fn delete_pool / DeletePool (name: String) -> Result<(), Refused>;
    /// A partition's reference codes, oldest first.
    16 => fn history / History (id: u32) -> Result<Vec<Posted>, Refused>;
    /// The host's settings for its partitions.
    17 => fn settings / GetSettings () -> Settings;
    /// Change the settings. The limit and the block size apply at
    /// once to what is checked from then on; the reserve applies at
    /// the host's next IPL. The pools are not changed by this.
    18 => fn change_settings / ChangeSettings (settings: Settings) -> Result<(), Refused>;
    /// End every partition, in the reverse of the order they were
    /// activated in: PID 1's request before the host stops. How many
    /// are being ended; a list shows when they have.
    19 => fn end_all / EndAll () -> u32;
    /// Every event from now on follows on this connection.
    20 => fn subscribe / Subscribe () -> ();
}

/// The socket of the partition manager whose run directory is `run_dir`.
pub fn socket(run_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(run_dir).join(SOCKET_FILE)
}

/// A client: each call is its own connection.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct Client {
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl Client {
    /// A client of the socket at `path`. Nothing is connected yet.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// One request, encoded, and its encoded answer: for whoever
    /// carries requests it does not read.
    pub fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, ipc::WireError> {
        supervise::control::exchange(&self.path, request)
    }

    /// Subscribe: the connection every event then arrives on.
    pub fn events(&self) -> Result<Events, ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(&Request::Subscribe {}))?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        ipc::decode::<()>(&answer)?;
        Ok(Events { stream })
    }
}

#[cfg(unix)]
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        self.exchange(&ipc::encode(req))
    }
}

/// The longest request, answer or event.
pub const MAX_FRAME: usize = 256 * 1024;

/// A subscription.
#[cfg(unix)]
#[derive(Debug)]
pub struct Events {
    stream: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl Events {
    /// The next event; waits for one, for `wait` at most when given.
    pub fn next(&mut self, wait: Option<std::time::Duration>) -> Result<Event, ipc::WireError> {
        self.stream.set_read_timeout(wait)?;
        let frame = ipc::frame::read_frame_max(&mut self.stream, MAX_FRAME)?;
        ipc::decode(&frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Units;
    use resource::Processors;

    #[test]
    fn a_status_and_an_event_round_trip() {
        let current = Current {
            processors: Processors::Shared {
                pool: "DEFAULT".into(),
                units: Units(50),
                virtuals: 1,
                capped: true,
                weight: 0,
            },
            memory_mib: 2048,
        };
        let status = Status {
            id: 3,
            name: "PROD".into(),
            state: State::Running,
            code: "C2005090".into(),
            text: "Partition started: PROD, side A".into(),
            current: Some(current.clone()),
            profile: "NORMAL".into(),
            last_end: String::new(),
            message: Some(qvsmsg::POOL_MISSING.with(&["X"])),
            pid: 412,
        };
        let back: Status = ipc::decode(&ipc::encode(&status)).unwrap();
        assert_eq!(back, status);
        for event in [
            Event::State {
                id: 3,
                name: "PROD".into(),
                state: State::ShuttingDown,
                time: 7,
            },
            Event::Code {
                id: 3,
                name: "PROD".into(),
                posted: Posted {
                    time: 7,
                    code: "C2005100".into(),
                    text: "Partition powered off".into(),
                },
            },
            Event::Resources {
                id: 3,
                name: "PROD".into(),
                current: Some(current),
                time: 8,
            },
            Event::Defined {
                id: 3,
                name: "PROD".into(),
                exists: false,
                time: 9,
            },
        ] {
            let back: Event = ipc::decode(&ipc::encode(&event)).unwrap();
            assert_eq!(back, event);
            assert_eq!(back.partition(), 3);
        }
    }

    #[test]
    fn the_states_are_the_names_shown_and_three_of_them_hold_resources() {
        let all = [
            State::NotActivated,
            State::Starting,
            State::Running,
            State::ShuttingDown,
            State::Error,
        ];
        let words: Vec<&str> = all.iter().map(|s| s.word()).collect();
        assert_eq!(
            words,
            [
                "Not activated",
                "Starting",
                "Running",
                "Shutting down",
                "Error"
            ]
        );
        let active: Vec<bool> = all.iter().map(|s| s.active()).collect();
        assert_eq!(active, [false, true, true, true, false]);
        assert_eq!(Request::EndAll {}.name(), "EndAll");
    }
}
