// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The storage service's socket: one request and one answer per
//! connection, each an `ipc` frame, as PID 1's control socket. The
//! service itself is the binary; this is what its callers need.

#![forbid(unsafe_code)]

#[cfg(unix)]
use std::path::{Path, PathBuf};

/// The service's socket.
pub const SOCKET: &str = "/run/qsvsstgd.sock";
/// Where the service says what `/data` is to be mounted from, before
/// it says it is ready: a device node, or nothing when it has none.
pub const DATA_DEVICE: &str = "/run/qsvsstgd.data";
/// The service's name in the service table.
pub const NAME: &str = "stgd";

/// `protection` of a load source with one disk.
pub const NONE: &str = "none";
/// `protection` of a mirrored load source.
pub const MIRRORED: &str = "mirrored";

/// One load-source unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    /// Its number, from 1.
    pub number: u32,
    /// The disk's kernel name, empty when the disk is not there.
    pub disk: String,
    /// The disk's serial number.
    pub serial: String,
    /// The disk's location code.
    pub location: String,
    /// The disk's size, 0 when it is not there.
    pub bytes: u64,
    /// `Unprotected`, `Active`, `Suspended`, `Resuming`, `Failed` or
    /// `Missing`.
    pub state: String,
}
ipc::wire_struct!(Unit {
    number,
    disk,
    serial,
    location,
    bytes,
    state
});

/// The load source, as status shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// [`NONE`] or [`MIRRORED`].
    pub protection: String,
    /// What `/data` is mounted from.
    pub device: String,
    /// The units, in order.
    pub units: Vec<Unit>,
    /// What the service is doing to a unit: empty, `preparing`,
    /// `resynchronizing` or `rebuilding`.
    pub work: String,
    /// The unit it is doing it to, from 1; 0 for none.
    pub work_unit: u32,
    /// How far that is, in percent.
    pub percent: u32,
    /// Mirrored protection was started and takes effect at the next IPL.
    pub restart_needed: bool,
    /// Why the last request failed after it was accepted; empty when
    /// it did not.
    pub failure: String,
}
ipc::wire_struct!(Status {
    protection,
    device,
    units,
    work,
    work_unit,
    percent,
    restart_needed,
    failure
});

impl Status {
    /// Whether every unit holds the data: one unprotected unit, or a
    /// mirrored pair in step.
    pub fn whole(&self) -> bool {
        self.units
            .iter()
            .all(|u| u.state == "Active" || u.state == "Unprotected")
    }
}

ipc::rpc! {
    request Request, service Storage, client Calls, dispatch dispatch;
    /// The load source.
    0 => fn status / GetStatus () -> Status;
    /// Start mirrored protection of an unprotected load source with
    /// the disk named. The disk is erased. It takes effect at the next
    /// IPL. `Ok` when the work was accepted; status says how it went.
    1 => fn start_protection / StartProtection (disk: String) -> Result<(), String>;
    /// Put the disk named in the place of unit `number`, which is
    /// missing or failed, and rebuild it. The disk is erased.
    2 => fn replace / Replace (number: u32, disk: String) -> Result<(), String>;
}

/// A client of the service's socket.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct Client {
    path: PathBuf,
}

#[cfg(unix)]
impl Client {
    /// A client of the socket at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The socket's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(unix)]
impl Default for Client {
    fn default() -> Self {
        Self::new(SOCKET)
    }
}

#[cfg(unix)]
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        supervise::control::exchange(&self.path, &ipc::encode(req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_and_says_whether_the_load_source_is_whole() {
        let unit = |number, state: &str| Unit {
            number,
            disk: "vda".into(),
            serial: "QSVSHD0".into(),
            location: "P0000-B00-D05-F0-U0".into(),
            bytes: 1 << 30,
            state: state.into(),
        };
        let mut status = Status {
            protection: MIRRORED.into(),
            device: "/dev/ublkb0".into(),
            units: vec![unit(1, "Active"), unit(2, "Resuming")],
            work: "rebuilding".into(),
            work_unit: 2,
            percent: 40,
            restart_needed: false,
            failure: String::new(),
        };
        let back: Status = ipc::decode(&ipc::encode(&status)).unwrap();
        assert_eq!(back, status);
        assert!(!status.whole());
        status.units[1].state = "Active".into();
        assert!(status.whole());
        assert!(
            Status {
                units: vec![unit(1, "Unprotected")],
                ..Status::default()
            }
            .whole()
        );
    }
}
