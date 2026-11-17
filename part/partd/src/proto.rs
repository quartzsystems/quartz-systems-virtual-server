// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager's socket (`docs/PARTD.md`): one request and
//! one answer per connection, each an `ipc` frame, as PID 1's control
//! socket. A request that fails answers with `QVS` messages. One
//! request is different: after `subscribe` is answered the connection
//! stays open and every event follows on it, a frame each.

use model::{Definition, IplSource, IplType, Keylock, Profile};
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
    /// Its VMM runs and its system has not reported its IPL complete.
    Starting,
    /// The system's IPL is complete, or it has no service channel and
    /// its kernel was entered.
    Running,
    /// A shut down was asked for, or the system is ending, and its VMM
    /// has not ended yet.
    ShuttingDown,
    /// Held with its memory after a terminal code or a guest fault (its
    /// VMM is kept and it holds what it held), or its VMM failed or was
    /// killed (it holds nothing and can be activated again).
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
    /// Its virtual Ethernet adapters: the running profile's, or the
    /// default profile's when it is not activated.
    pub ethernet: Vec<Ethernet>,
    /// Its service channel and what the system reported of itself.
    pub service: Service,
}

/// A partition's service channel, and what the system reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // four facts apart
pub struct Service {
    /// Whether the channel is up: the system said hello on it.
    pub up: bool,
    /// Whether the system has none: its kernel was entered and it did
    /// not open the service port in time.
    pub none: bool,
    /// Whether it answers pings.
    pub responding: bool,
    /// Whether it is held with its memory.
    pub kept: bool,
    /// The system's name.
    pub system: String,
    /// Its release, `V1R2M0`; empty until known.
    pub release: String,
    /// Its fix level; empty until known.
    pub fix_level: String,
    /// Its addresses, each `address/prefix`.
    pub addresses: Vec<String>,
    /// The protocol version both sides speak; 0 with no channel.
    pub version: u16,
    /// The capabilities both have.
    pub caps: u64,
}
ipc::wire_struct!(Service {
    up,
    none,
    responding,
    kept,
    system,
    release,
    fix_level,
    addresses,
    version,
    caps
});

/// A virtual Ethernet adapter, as a list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ethernet {
    /// Its slot.
    pub slot: u32,
    /// Its switch.
    pub switch: String,
    /// Its MAC address.
    pub mac: String,
    /// Whether it has link: its port is open on a running partition.
    pub link: bool,
}
ipc::wire_struct!(Ethernet {
    slot,
    switch,
    mac,
    link
});

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
    pid,
    ethernet,
    service
});

/// Who posted a reference code of a partition's history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The host, about the partition (`docs/REFCODES.md`).
    #[default]
    Host,
    /// The system in it, on its service channel.
    System,
}
ipc::wire_enum!(Source { 0 => Host, 1 => System });

impl Source {
    /// As a history shows it.
    pub fn word(self) -> &'static str {
        match self {
            Source::Host => "host",
            Source::System => "system",
        }
    }
}

/// One reference code of a partition's history.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Posted {
    /// When, in seconds since the epoch.
    pub time: u64,
    /// The code.
    pub code: String,
    /// Its text.
    pub text: String,
    /// Who posted it.
    pub source: Source,
    /// Its words 2 to 9, as the system gave them; none for a host code.
    pub words: Vec<String>,
}
ipc::wire_struct!(Posted {
    time,
    code,
    text,
    source,
    words
});

/// Something that happened to a partition.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)] // made once and sent: a device's whole view
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
    /// Its service channel came up or went, it stopped or started
    /// answering, or the system reported a change of itself.
    Channel {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// The channel as it is now.
        service: Service,
        /// When.
        time: u64,
    },
    /// A dump of it was written, or could not be.
    Dump {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// The dump's name.
        dump: String,
        /// Why it was not written; none when it was.
        error: Option<Issued>,
        /// When.
        time: u64,
    },
    /// One of its profiles was put, deleted or made the default.
    Profile {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// The profile.
        profile: String,
        /// Its revision now; none when it was deleted.
        revision: Option<u64>,
        /// Whether it is the partition's default now.
        default: bool,
        /// When.
        time: u64,
    },
    /// A running partition's entitlement changed.
    Entitlement {
        /// The partition.
        id: u32,
        /// Its name.
        name: String,
        /// What it held before.
        before: Current,
        /// What it holds now.
        current: Current,
        /// Who asked, in the asker's words.
        by: String,
        /// When.
        time: u64,
    },
    /// A device of physical I/O changed hands.
    Device {
        /// Its location code.
        location: String,
        /// What happened.
        change: DeviceChange,
        /// The partition it was given to or given back by; 0 for none.
        partition: u32,
        /// That partition's name.
        name: String,
        /// The profile that partition was activated with.
        profile: String,
        /// The device as it is now; none when it is not on the host.
        device: Option<pio::Device>,
        /// Why it was fenced, for a reset that failed.
        note: Option<Issued>,
        /// When.
        time: u64,
    },
}
ipc::wire_enum!(Event {
    0 => State { id, name, state, time },
    1 => Code { id, name, posted },
    2 => Resources { id, name, current, time },
    3 => Defined { id, name, exists, time },
    4 => Channel { id, name, service, time },
    5 => Dump { id, name, dump, error, time },
    6 => Profile { id, name, profile, revision, default, time },
    7 => Device { location, change, partition, name, profile, device, note, time },
    8 => Entitlement { id, name, before, current, by, time },
});

/// What happened to a device of physical I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceChange {
    /// It was released for partitions at the host.
    Released,
    /// It was taken back at the host.
    TakenBack,
    /// It was given to a partition at its activation.
    Assigned,
    /// It was given back at the partition's end, reset.
    Returned,
    /// It did not reset, and is fenced.
    ResetFailed,
}
ipc::wire_enum!(DeviceChange {
    0 => Released,
    1 => TakenBack,
    2 => Assigned,
    3 => Returned,
    4 => ResetFailed,
});

impl DeviceChange {
    /// As the log says it.
    pub fn word(self) -> &'static str {
        match self {
            DeviceChange::Released => "released",
            DeviceChange::TakenBack => "taken back",
            DeviceChange::Assigned => "given",
            DeviceChange::Returned => "given back",
            DeviceChange::ResetFailed => "fenced, its reset failed",
        }
    }
}

impl Event {
    /// The partition it is about.
    pub fn partition(&self) -> u32 {
        match self {
            Event::State { id, .. }
            | Event::Code { id, .. }
            | Event::Resources { id, .. }
            | Event::Defined { id, .. }
            | Event::Channel { id, .. }
            | Event::Dump { id, .. }
            | Event::Profile { id, .. }
            | Event::Entitlement { id, .. } => *id,
            Event::Device { partition, .. } => *partition,
        }
    }
}

/// One profile of a partition, as listed with its revision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileEntry {
    /// Its name.
    pub name: String,
    /// Its revision.
    pub revision: u64,
    /// Whether it is the partition's default.
    pub default: bool,
}
ipc::wire_struct!(ProfileEntry {
    name,
    revision,
    default
});

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
    /// activated. Its virtual disks are deleted with it when `disks`,
    /// and kept otherwise.
    4 => fn delete / Delete (id: u32, disks: bool) -> Result<(), Refused>;
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
    /// Load the image or mount the volume `name` of the media library
    /// into the optical or tape device in `slot` of a running partition.
    21 => fn load_media / LoadMedia (id: u32, slot: u32, name: String) -> Result<(), Refused>;
    /// Unload the medium of the device in `slot`; when the partition
    /// prevents its removal, only `forced`. The medium's name.
    22 => fn unload_media / UnloadMedia (id: u32, slot: u32, forced: bool) -> Result<String, Refused>;
    /// The optical and tape devices of a running partition, and what
    /// each has.
    23 => fn media / Media (id: u32) -> Result<Vec<MediaSlot>, Refused>;
    /// The profiles that name the virtual disk `name` of `pool`: each
    /// as `PROD profile NORMAL slot 2`.
    24 => fn named / Named (pool: String, name: String) -> Vec<String>;
    /// Every virtual Ethernet adapter of a partition's profiles, with its
    /// switch and MAC address; of every partition with 0.
    25 => fn adapters / Adapters (id: u32) -> Vec<vswitchd::Adapter>;
    /// Shut a partition down by `kind`, with `delay` for a delayed shut
    /// down, activating it again when `restart`. Answers when the system
    /// has accepted it, or at once when immediate.
    26 => fn shut_down_as / ShutDownAs (id: u32, kind: Shutdown, delay: u32, restart: bool) -> Result<(), Refused>;
    /// A control panel function, with its arguments: the text it
    /// displays, empty when it displays nothing.
    27 => fn panel / Panel (id: u32, function: u8, args: PanelArgs) -> Result<String, Refused>;
    /// Dump a running or held partition, without restarting it. The
    /// dump's name; it is written in the background, and an event says
    /// when it is done.
    28 => fn take_dump / TakeDump (id: u32) -> Result<String, Refused>;
    /// A partition's dumps, oldest first.
    29 => fn dumps / Dumps (id: u32) -> Result<Vec<DumpInfo>, Refused>;
    /// Delete a dump.
    30 => fn delete_dump / DeleteDump (id: u32, name: String) -> Result<(), Refused>;
    /// Read a dump: its size, and then its bytes on this connection, a
    /// frame of at most [`DUMP_PIECE`] each, to the end.
    31 => fn read_dump / ReadDump (id: u32, name: String) -> Result<u64, Refused>;
    /// Attach to a partition's console, taking it over from another
    /// viewer when `take_over`: whether it is the serial port's bytes.
    /// Then [`ConsoleOut`] follows on this connection, and the client
    /// sends [`ConsoleIn`], until either side closes it.
    32 => fn console / Console (id: u32, take_over: bool) -> Result<bool, Refused>;
    /// The console's screen as it is now, alone.
    33 => fn screen / Screen (id: u32) -> Result<ScreenCapture, Refused>;
    /// Put a profile only when it is at `revision`: `None` for a new
    /// one, refused when one of its name is there. Its new revision.
    34 => fn put_profile_at / PutProfileAt (id: u32, profile: Profile, revision: Option<u64>) -> Result<u64, Refused>;
    /// Save an activated partition's current configuration (the
    /// profile it runs with, with what its activation gave it) as the
    /// profile `name`, at `revision` as [`Calls::put_profile_at`]. Its
    /// new revision.
    35 => fn save_current / SaveCurrent (id: u32, name: String, revision: Option<u64>) -> Result<u64, Refused>;
    /// Create a partition called `name` with its first profile, which
    /// is its default; when the profile is refused, nothing is kept.
    36 => fn create_with / CreateWith (name: String, profile: Profile) -> Result<Definition, Refused>;
    /// Change a definition only when it is at `revision`. Its new
    /// revision.
    37 => fn change_at / ChangeAt (definition: Definition, revision: u64) -> Result<u64, Refused>;
    /// A partition's profiles with their revisions, by name.
    38 => fn profile_entries / ProfileEntries (id: u32) -> Result<Vec<ProfileEntry>, Refused>;
    /// The host's PCI devices as physical I/O sees them, by location
    /// code (`docs/PHYSICALIO.md`).
    39 => fn devices / Devices () -> Vec<pio::Device>;
    /// Release the device at `location` and the rest of its isolation
    /// group for partitions, `confirm` the location typed in full. The
    /// locations released.
    40 => fn release / Release (location: String, confirm: String) -> Result<Vec<String>, Refused>;
    /// Take the device at `location` and its group back for the host,
    /// when no profile names any of them. The locations taken back.
    41 => fn take_back / TakeBack (location: String) -> Result<Vec<String>, Refused>;
    /// Change a running shared partition's processing units, capping or
    /// weight (`docs/RESOURCES.md`); `by` says who asked. What it holds
    /// now. Its profile is not changed.
    42 => fn entitlement / ChangeEntitlement (id: u32, change: resource::Entitlement, by: String) -> Result<Current, Refused>;
}

/// The most bytes of a dump in one frame of a read.
pub const DUMP_PIECE: usize = 128 * 1024;

/// How a partition is shut down (`docs/SERVICE.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shutdown {
    /// The system's controlled power down, with a delay; forced when the
    /// delay and a grace period have passed.
    Delayed,
    /// The system's immediate power down; forced after a timeout.
    System,
    /// The VMM is stopped.
    Immediate,
}
ipc::wire_enum!(Shutdown { 0 => Delayed, 1 => System, 2 => Immediate });

impl Shutdown {
    /// As a message names it.
    pub fn word(self) -> &'static str {
        match self {
            Shutdown::Delayed => "delayed",
            Shutdown::System => "operating system",
            Shutdown::Immediate => "immediate",
        }
    }
}

/// Function 02's selection for the next activation; what is left out
/// is not changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PanelArgs {
    /// The IPL source.
    pub source: Option<IplSource>,
    /// The IPL type.
    pub ipl_type: Option<IplType>,
    /// The keylock position.
    pub keylock: Option<Keylock>,
}
ipc::wire_struct!(PanelArgs {
    source,
    ipl_type,
    keylock
});

/// One dump of a partition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DumpInfo {
    /// Its name.
    pub name: String,
    /// When it was taken, in seconds since the epoch.
    pub time: u64,
    /// Its size.
    pub bytes: u64,
    /// The bytes it takes in the pool.
    pub allocated: u64,
    /// The partition's reference code when it was taken.
    pub code: String,
    /// Why it was taken.
    pub reason: String,
}
ipc::wire_struct!(DumpInfo {
    name,
    time,
    bytes,
    allocated,
    code,
    reason
});

/// A partition's console screen, captured.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenCapture {
    /// Whether the console is the serial port's: there is then no frame.
    pub serial: bool,
    /// The whole frame.
    pub frame: Option<qsipart::console::Frame>,
    /// The screen as text, a line a row.
    pub text: String,
}
ipc::wire_struct!(ScreenCapture {
    serial,
    frame,
    text
});

/// What an attached console's client is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleOut {
    /// A whole frame: the first after attaching, and when the system
    /// sent one.
    Frame {
        /// It.
        frame: qsipart::console::Frame,
    },
    /// What changed.
    Update {
        /// It.
        update: qsipart::console::Update,
    },
    /// The serial port's bytes, when that is the console.
    Serial {
        /// They.
        bytes: Vec<u8>,
    },
    /// The client is detached: taken over, or the partition ended.
    Ended {
        /// Why.
        why: String,
    },
}
ipc::wire_enum!(ConsoleOut {
    0 => Frame { frame },
    1 => Update { update },
    2 => Serial { bytes },
    3 => Ended { why },
});

/// What an attached console's client sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleIn {
    /// A key, for the console port.
    Key {
        /// It.
        key: qsipart::console::Key,
    },
    /// Ask for a whole frame.
    Refresh,
    /// Bytes for the serial port, when that is the console.
    Bytes {
        /// They.
        bytes: Vec<u8>,
    },
}
ipc::wire_enum!(ConsoleIn {
    0 => Key { key },
    1 => Refresh,
    2 => Bytes { bytes },
});

/// An optical or tape device of a running partition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaSlot {
    /// Its slot.
    pub slot: u32,
    /// `optical` or `tape`.
    pub kind: String,
    /// The medium's name; empty for none.
    pub name: String,
    /// Whether the partition prevents its removal.
    pub prevented: bool,
}
ipc::wire_struct!(MediaSlot {
    slot,
    kind,
    name,
    prevented
});

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
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        stream.set_read_timeout(Some(ANSWER_WAIT))?;
        stream.set_write_timeout(Some(ANSWER_WAIT))?;
        ipc::frame::write_frame(&mut stream, request)?;
        ipc::frame::read_frame_max(&mut stream, MAX_FRAME)
    }

    /// Attach to `id`'s console, taking it over when `take_over`:
    /// whether it is the serial port's, and the connection.
    pub fn console(
        &self,
        id: u32,
        take_over: bool,
    ) -> Result<Result<(bool, ConsoleSession), Refused>, ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        stream.set_read_timeout(Some(ANSWER_WAIT))?;
        ipc::frame::write_frame(
            &mut stream,
            &ipc::encode(&Request::Console { id, take_over }),
        )?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        let attached: Result<bool, Refused> = ipc::decode(&answer)?;
        Ok(attached.map(|serial| (serial, ConsoleSession { stream })))
    }

    /// Read `id`'s dump `name` into `out`: its size.
    pub fn read_dump_into(
        &self,
        id: u32,
        name: &str,
        out: &mut dyn std::io::Write,
    ) -> Result<Result<u64, Refused>, ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        stream.set_read_timeout(Some(ANSWER_WAIT))?;
        let request = Request::ReadDump {
            id,
            name: name.to_string(),
        };
        ipc::frame::write_frame(&mut stream, &ipc::encode(&request))?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        let size: Result<u64, Refused> = ipc::decode(&answer)?;
        if size.is_ok() {
            loop {
                let piece = ipc::frame::read_frame_max(&mut stream, DUMP_PIECE)?;
                if piece.is_empty() {
                    break;
                }
                out.write_all(&piece)?;
            }
        }
        Ok(size)
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

/// How long a client waits for an answer. A request that changes a
/// definition, a profile or the settings writes a state file to the
/// disk before it answers, which on a busy host can take seconds.
#[cfg(unix)]
pub const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// An attached console's connection.
#[cfg(unix)]
#[derive(Debug)]
pub struct ConsoleSession {
    stream: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl ConsoleSession {
    /// Send keys, a refresh, or the serial port's bytes.
    pub fn send(&mut self, message: &ConsoleIn) -> Result<(), ipc::WireError> {
        ipc::frame::write_frame(&mut self.stream, &ipc::encode(message))
    }

    /// The next message for the viewer; waits `wait` at most.
    pub fn next(&mut self, wait: std::time::Duration) -> Result<ConsoleOut, ipc::WireError> {
        self.stream.set_read_timeout(Some(wait))?;
        let frame = ipc::frame::read_frame_max(&mut self.stream, MAX_FRAME)?;
        ipc::decode(&frame)
    }

    /// A second handle on the connection: one to read, one to send.
    pub fn try_clone(&self) -> std::io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
        })
    }

    /// Detach.
    pub fn close(self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

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
            ethernet: vec![Ethernet {
                slot: 5,
                switch: "PRODNET".into(),
                mac: "02:5a:3c:00:03:05".into(),
                link: true,
            }],
            service: Service {
                up: true,
                release: "V1R2M0".into(),
                addresses: vec!["10.0.2.15/24".into()],
                ..Service::default()
            },
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
                    ..Posted::default()
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
