// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The pool service's socket (`docs/POOLD.md`): one request and one
//! answer per connection, each an `ipc` frame, as the partition
//! manager's. A request that fails answers with every `QVS` message
//! that applies. Two requests carry more than frames:
//!
//! - **hold**: after an answer that gives backing stores, their open
//!   files follow on one byte (`base/fdpass`), in the answer's order.
//! - **upload**: after an `Ok` answer the client sends the image's
//!   bytes, exactly as many as it said, and the service answers again
//!   when they are checked and named.

use qvsmsg::Issued;

/// The service's name in the service table.
pub const NAME: &str = "poold";
/// The socket's file in the run directory.
pub const SOCKET_FILE: &str = "qsvspoold.sock";
/// The longest request or answer.
pub const MAX_FRAME: usize = 256 * 1024;

/// What a request that failed answers with: every message.
pub type Refused = Vec<Issued>;

/// The socket of the service whose run directory is `run_dir`.
pub fn socket(run_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(run_dir).join(SOCKET_FILE)
}

/// One disk of the host, as Display disk configuration shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    /// Its kernel name.
    pub name: String,
    /// Its serial number.
    pub serial: String,
    /// Its location code.
    pub location: String,
    /// Its size.
    pub bytes: u64,
    /// Its logical block size.
    pub logical_block: u32,
    /// What it is, in words: `Load source`, `POOL1 unit 2, Active`,
    /// `Hot spare, POOL1`, `Not configured`, `Foreign: ...`.
    pub what: String,
    /// The pool it is in, or a spare of; empty for none.
    pub pool: String,
    /// Whether a pool may be made of it without a confirmation.
    pub free: bool,
    /// Whether it holds what is not this host's: it is taken only with
    /// a typed confirmation.
    pub foreign: bool,
}
ipc::wire_struct!(Disk {
    name,
    serial,
    location,
    bytes,
    logical_block,
    what,
    pool,
    free,
    foreign
});

/// A unit of a pool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoolUnit {
    /// Its number, from 1; hot spares follow the members.
    pub number: u32,
    /// Its disk's kernel name; empty when it is not there.
    pub disk: String,
    /// Its disk's serial number.
    pub serial: String,
    /// Its disk's location code.
    pub location: String,
    /// Its disk's size.
    pub bytes: u64,
    /// `Active`, `Rebuilding`, `Resuming`, `Suspended`, `Failed`,
    /// `Missing` or `Hot spare`.
    pub state: String,
}
ipc::wire_struct!(PoolUnit {
    number,
    disk,
    serial,
    location,
    bytes,
    state
});

/// A pool's state.
pub mod state {
    /// Every unit in step.
    pub const ACTIVE: &str = "Active";
    /// A unit failed or is missing; the pool runs.
    pub const DEGRADED: &str = "Degraded";
    /// A unit is being rebuilt.
    pub const REBUILDING: &str = "Rebuilding";
    /// Its units are being brought in step after an unclean stop, or a
    /// new parity set's parity is being computed.
    pub const RESYNCHRONIZING: &str = "Resynchronizing";
    /// More units are lost than its protection covers.
    pub const FAILED: &str = "Failed";
    /// Its units are being found.
    pub const ASSEMBLING: &str = "Assembling";
}

/// A pool, as a list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoolStatus {
    /// Its name.
    pub name: String,
    /// Its identity.
    pub uuid: String,
    /// `none`, `mirrored`, `raid5` or `raid6`.
    pub protection: String,
    /// One of [`state`].
    pub state: String,
    /// What it is doing: empty, `rebuilding`, `resynchronizing`.
    pub work: String,
    /// The unit worked on, from 1; 0 for the whole pool.
    pub work_unit: u32,
    /// How far, in percent.
    pub percent: u32,
    /// Its size in bytes: what its file system holds.
    pub bytes: u64,
    /// Bytes allocated in it.
    pub allocated: u64,
    /// Bytes its virtual disks are, thin or thick, and its media.
    pub provisioned: u64,
    /// Where it is mounted; empty when it is not.
    pub mount: String,
    /// Whether it holds the media library.
    pub repository: bool,
    /// Its units.
    pub units: Vec<PoolUnit>,
    /// Why it failed or was not assembled; empty when it did not.
    pub failure: String,
}
ipc::wire_struct!(PoolStatus {
    name,
    uuid,
    protection,
    state,
    work,
    work_unit,
    percent,
    bytes,
    allocated,
    provisioned,
    mount,
    repository,
    units,
    failure
});

impl PoolStatus {
    /// Whether a partition may use it: Active, Degraded, or working.
    pub fn usable(&self) -> bool {
        !matches!(self.state.as_str(), state::FAILED | state::ASSEMBLING | "")
            && !self.mount.is_empty()
    }
}

/// Who holds a backing store or a medium.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Holder {
    /// The partition's ID; 0 for none.
    pub partition: u32,
    /// The slot it is in.
    pub slot: u32,
}
ipc::wire_struct!(Holder { partition, slot });

/// A virtual disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vdisk {
    /// Its pool.
    pub pool: String,
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Bytes the pool has given it.
    pub allocated: u64,
    /// Whether it is thick: allocated whole when made.
    pub thick: bool,
    /// Who holds it.
    pub holder: Holder,
}
ipc::wire_struct!(Vdisk {
    pool,
    name,
    bytes,
    allocated,
    thick,
    holder
});

/// An optical image of the media library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Image {
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Its volume identifier.
    pub label: String,
    /// Who has it loaded.
    pub holder: Holder,
}
ipc::wire_struct!(Image {
    name,
    bytes,
    label,
    holder
});

/// A tape volume of the media library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    /// Its name.
    pub name: String,
    /// Its volume identifier.
    pub identifier: String,
    /// Its capacity in MiB.
    pub capacity_mib: u64,
    /// Bytes of its image in use.
    pub used: u64,
    /// Whether it is write-protected.
    pub protected: bool,
    /// Who has it mounted.
    pub holder: Holder,
}
ipc::wire_struct!(Volume {
    name,
    identifier,
    capacity_mib,
    used,
    protected,
    holder
});

/// What a hold asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A virtual disk.
    #[default]
    Vdisk,
    /// An optical image.
    Image,
    /// A tape volume.
    Volume,
}
ipc::wire_enum!(Kind {
    0 => Vdisk,
    1 => Image,
    2 => Volume,
});

/// One thing a partition is to hold.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// What it is.
    pub kind: Kind,
    /// A virtual disk's pool; empty for media.
    pub pool: String,
    /// Its name.
    pub name: String,
    /// The slot it is for.
    pub slot: u32,
}
ipc::wire_struct!(Want {
    kind,
    pool,
    name,
    slot
});

/// One thing given: its open file follows the answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Given {
    /// What was asked for.
    pub want: Want,
    /// Its size.
    pub bytes: u64,
    /// A tape volume's capacity in MiB.
    pub capacity_mib: u64,
    /// A tape volume's write protection.
    pub protected: bool,
}
ipc::wire_struct!(Given {
    want,
    bytes,
    capacity_mib,
    protected
});

/// A held item, as listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// The partition.
    pub partition: u32,
    /// What it holds.
    pub want: Want,
}
ipc::wire_struct!(Held { partition, want });

ipc::rpc! {
    request Request, service Pools, client Calls, dispatch dispatch;
    /// Every disk of the host and what it is.
    0 => fn disks / Disks () -> Vec<Disk>;
    /// Every pool.
    1 => fn pools / Pools () -> Vec<PoolStatus>;
    /// Make a pool of `members` with `protection` and hot `spares`.
    /// A disk with what is not this host's on it is taken only when it
    /// is named in `confirm`. The disks are erased.
    2 => fn create_pool / CreatePool (name: String, protection: String, members: Vec<String>, spares: Vec<String>, confirm: Vec<String>) -> Result<PoolStatus, Refused>;
    /// Delete a pool that holds no virtual disk and no media.
    3 => fn delete_pool / DeletePool (name: String) -> Result<(), Refused>;
    /// Rename a pool.
    4 => fn rename_pool / RenamePool (name: String, to: String) -> Result<(), Refused>;
    /// Put `disk` in the place of the pool's failed or missing unit
    /// `unit` and rebuild it.
    5 => fn replace_unit / ReplaceUnit (pool: String, unit: u32, disk: String, confirm: bool) -> Result<(), Refused>;
    /// Give a pool a hot spare.
    6 => fn add_spare / AddSpare (pool: String, disk: String, confirm: bool) -> Result<(), Refused>;
    /// Take a hot spare from a pool.
    7 => fn remove_spare / RemoveSpare (pool: String, disk: String) -> Result<(), Refused>;
    /// Make `pool` the media repository; empty for none.
    8 => fn designate / Designate (pool: String) -> Result<(), Refused>;
    /// The virtual disks of a pool; every pool's when empty.
    9 => fn vdisks / Vdisks (pool: String) -> Vec<Vdisk>;
    /// Make a virtual disk of `mib`, thick or thin.
    10 => fn create_vdisk / CreateVdisk (pool: String, name: String, mib: u64, thick: bool) -> Result<Vdisk, Refused>;
    /// Grow a virtual disk no running partition has.
    11 => fn resize_vdisk / ResizeVdisk (pool: String, name: String, mib: u64) -> Result<Vdisk, Refused>;
    /// Delete a virtual disk no partition holds. One a profile of a
    /// partition that is not activated names is deleted only when
    /// `force`.
    12 => fn delete_vdisk / DeleteVdisk (pool: String, name: String, force: bool) -> Result<(), Refused>;
    /// The optical images.
    13 => fn images / Images () -> Vec<Image>;
    /// Upload image `name` of `bytes`: the bytes follow an `Ok`.
    14 => fn upload / Upload (name: String, bytes: u64) -> Result<(), Refused>;
    /// Rename an image.
    15 => fn rename_image / RenameImage (name: String, to: String) -> Result<(), Refused>;
    /// Delete an image no partition has loaded.
    16 => fn delete_image / DeleteImage (name: String) -> Result<(), Refused>;
    /// Import image `name` from a removable volume or optical drive of
    /// the host: `device` is its kernel name, `path` a file on it or
    /// empty for the whole of an optical disc. The pools' list says
    /// how it goes.
    17 => fn import_image / ImportImage (device: String, path: String, name: String) -> Result<(), Refused>;
    /// The tape volumes.
    18 => fn volumes / Volumes () -> Vec<Volume>;
    /// Make a tape volume.
    19 => fn create_volume / CreateVolume (name: String, identifier: String, mib: u64) -> Result<Volume, Refused>;
    /// Delete a tape volume no partition has mounted.
    20 => fn delete_volume / DeleteVolume (name: String) -> Result<(), Refused>;
    /// Set or clear a tape volume's write protection.
    21 => fn protect_volume / ProtectVolume (name: String, on: bool) -> Result<(), Refused>;
    /// Have `partition` hold what it wants: the open files follow.
    22 => fn hold / Hold (partition: u32, wants: Vec<Want>) -> Result<Vec<Given>, Refused>;
    /// Give back what `partition` holds: the items named, or all.
    23 => fn release / Release (partition: u32, wants: Vec<Want>) -> ();
    /// Everything held.
    24 => fn holders / Holders () -> Vec<Held>;
    /// Stop every pool in order: PID 1's request at the host's
    /// shutdown, after every partition ended.
    25 => fn stop_all / StopAll () -> u32;
}

/// A client: each call is its own connection.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct Client {
    path: std::path::PathBuf,
}

/// How long a client waits for an answer: making a pool formats its
/// file system.
pub const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

#[cfg(unix)]
impl Client {
    /// A client of the socket at `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn connect(&self) -> Result<std::os::unix::net::UnixStream, ipc::WireError> {
        let stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        stream.set_read_timeout(Some(ANSWER_WAIT))?;
        stream.set_write_timeout(Some(ANSWER_WAIT))?;
        Ok(stream)
    }

    /// One request, encoded, and its encoded answer.
    pub fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, ipc::WireError> {
        let mut stream = self.connect()?;
        ipc::frame::write_frame(&mut stream, request)?;
        ipc::frame::read_frame_max(&mut stream, MAX_FRAME)
    }

    /// Hold `wants` for `partition`: what was given, each with its open
    /// file, in order.
    pub fn hold_files(
        &self,
        partition: u32,
        wants: Vec<Want>,
    ) -> Result<Result<Vec<(Given, std::fs::File)>, Refused>, ipc::WireError> {
        let mut stream = self.connect()?;
        let request = Request::Hold { partition, wants };
        ipc::frame::write_frame(&mut stream, &ipc::encode(&request))?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        let given: Result<Vec<Given>, Refused> = ipc::decode(&answer)?;
        let Ok(given) = given else {
            return Ok(given.map(|_| Vec::new()));
        };
        if given.is_empty() {
            return Ok(Ok(Vec::new()));
        }
        let fds = fdpass::recv(&stream, given.len())?;
        if fds.len() != given.len() {
            return Err(ipc::WireError::from(std::io::Error::other(
                "fewer files than were given",
            )));
        }
        Ok(Ok(given
            .into_iter()
            .zip(fds.into_iter().map(std::fs::File::from))
            .collect()))
    }

    /// Upload image `name` from `source`, of `bytes`: the image as it
    /// was named.
    pub fn upload_from(
        &self,
        name: &str,
        bytes: u64,
        source: &mut dyn std::io::Read,
    ) -> Result<Result<Image, Refused>, ipc::WireError> {
        let mut stream = self.connect()?;
        let request = Request::Upload {
            name: name.to_string(),
            bytes,
        };
        ipc::frame::write_frame(&mut stream, &ipc::encode(&request))?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        let accepted: Result<(), Refused> = ipc::decode(&answer)?;
        if let Err(refused) = accepted {
            return Ok(Err(refused));
        }
        std::io::copy(&mut std::io::Read::take(source, bytes), &mut stream)?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        ipc::decode(&answer)
    }
}

#[cfg(unix)]
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        self.exchange(&ipc::encode(req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_socket_carries_round_trips() {
        let status = PoolStatus {
            name: "POOL1".into(),
            protection: "mirrored".into(),
            state: state::DEGRADED.into(),
            mount: "/pools/POOL1".into(),
            units: vec![PoolUnit {
                number: 1,
                state: "Missing".into(),
                ..PoolUnit::default()
            }],
            ..PoolStatus::default()
        };
        let back: PoolStatus = ipc::decode(&ipc::encode(&status)).unwrap();
        assert_eq!(back, status);
        assert!(back.usable());
        let failed = PoolStatus {
            state: state::FAILED.into(),
            ..status
        };
        assert!(!failed.usable());
        let want = Want {
            kind: Kind::Volume,
            pool: String::new(),
            name: "SAVE1".into(),
            slot: 4,
        };
        let request = Request::Hold {
            partition: 3,
            wants: vec![want.clone()],
        };
        let back: Request = ipc::decode(&ipc::encode(&request)).unwrap();
        assert!(matches!(
            back,
            Request::Hold { partition: 3, wants } if wants == [want.clone()]
        ));
        let given = Given {
            want,
            bytes: 10,
            capacity_mib: 1024,
            protected: true,
        };
        let back: Given = ipc::decode(&ipc::encode(&given)).unwrap();
        assert_eq!(back, given);
    }
}
