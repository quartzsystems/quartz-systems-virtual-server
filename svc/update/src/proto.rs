// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The update service's socket (`docs/UPDATE.md`): a request a
//! connection and its answer, a stage whose bytes follow its first
//! answer, or a subscription whose events follow. Root's and the QSI
//! server's group's.

use qvsmsg::Issued;

/// The service's name in PID 1's table.
pub const NAME: &str = "updd";
/// The socket's file, in the run directory.
pub const SOCKET_FILE: &str = "qsvsupdd.sock";
/// The longest request, answer or event.
pub const MAX_FRAME: usize = 256 * 1024;

/// The socket in `run_dir`.
#[must_use]
pub fn socket(run_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(run_dir).join(SOCKET_FILE)
}

/// How a slot's last activation ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Attempt {
    /// Never activated: what it holds was installed and never tried,
    /// or it holds what the host was installed with.
    #[default]
    Never,
    /// Installed and pending.
    Pending,
    /// It started and confirmed.
    Confirmed,
    /// It did not confirm and the host went back.
    RolledBack,
}
ipc::wire_enum!(Attempt {
    0 => Never,
    1 => Pending,
    2 => Confirmed,
    3 => RolledBack,
});

/// What an update image is.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageInfo {
    /// The product's name.
    pub product: String,
    /// Its release.
    pub release: String,
    /// Its build.
    pub build: String,
    /// The fixes it names.
    pub fixes: Vec<String>,
    /// The image's size.
    pub bytes: u64,
}
ipc::wire_struct!(ImageInfo {
    product,
    release,
    build,
    fixes,
    bytes
});

/// One slot of the host's code.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    /// `A` or `B`.
    pub name: String,
    /// The release it holds; empty when not known.
    pub release: String,
    /// Its build; empty when not known.
    pub build: String,
    /// Its fixes.
    pub fixes: Vec<String>,
    /// The host runs from it.
    pub running: bool,
    /// It is the A/B record's default.
    pub default: bool,
    /// How its last activation ended.
    pub last: Attempt,
    /// Why it rolled back.
    pub reason: String,
    /// When its last change was, seconds since the epoch.
    pub time: u64,
}
ipc::wire_struct!(Slot {
    name,
    release,
    build,
    fixes,
    running,
    default,
    last,
    reason,
    time
});

/// Where a staged image is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Verification {
    /// Its bytes are coming.
    #[default]
    Receiving,
    /// Whole, and being checked.
    Verifying,
    /// It passed every check.
    Verified,
    /// It failed one.
    Refused,
}
ipc::wire_enum!(Verification {
    0 => Receiving,
    1 => Verifying,
    2 => Verified,
    3 => Refused,
});

/// The image staged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// The name it was staged by.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Where it is.
    pub verification: Verification,
    /// What it is, once verified.
    pub image: Option<ImageInfo>,
    /// Why it was refused.
    pub error: Vec<Issued>,
    /// When it was staged, seconds since the epoch.
    pub time: u64,
}
ipc::wire_struct!(Staged {
    name,
    bytes,
    verification,
    image,
    error,
    time
});

/// An install's progress.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// The slot written.
    pub slot: String,
    /// How much is done.
    pub percent: u8,
    /// When, seconds since the epoch.
    pub time: u64,
}
ipc::wire_struct!(Progress {
    slot,
    percent,
    time
});

/// The slots, what is staged and what is being installed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// Slot A, then slot B.
    pub slots: Vec<Slot>,
    /// What is staged.
    pub staged: Option<Staged>,
    /// An install under way.
    pub installing: Option<Progress>,
}
ipc::wire_struct!(Status {
    slots,
    staged,
    installing
});

/// What the update service says happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// An upload ended: whole and being verified, or why not.
    Staged {
        /// Its name.
        name: String,
        /// The bytes received.
        bytes: u64,
        /// Why not.
        error: Vec<Issued>,
        /// When.
        time: u64,
    },
    /// The image was verified, or refused.
    Verified {
        /// Its name.
        name: String,
        /// What it is, when verified.
        image: Option<ImageInfo>,
        /// Why it was refused.
        error: Vec<Issued>,
        /// When.
        time: u64,
    },
    /// An install's progress.
    Installing {
        /// Where it is.
        progress: Progress,
    },
    /// The slot is written and pending, or the install failed.
    Installed {
        /// The slot.
        slot: String,
        /// What it holds now, when installed.
        image: Option<ImageInfo>,
        /// Why not.
        error: Vec<Issued>,
        /// When.
        time: u64,
    },
    /// The restart into the pending slot begins.
    Activating {
        /// The slot.
        slot: String,
        /// When.
        time: u64,
    },
}
ipc::wire_enum!(Event {
    0 => Staged { name, bytes, error, time },
    1 => Verified { name, image, error, time },
    2 => Installing { progress },
    3 => Installed { slot, image, error, time },
    4 => Activating { slot, time },
});

ipc::rpc! {
    request Request, service Updates, client Calls, dispatch dispatch;
    /// The slots, what is staged, what is being installed.
    0 => fn status / Status () -> Status;
    /// Stage an image of exactly `bytes`: its bytes follow an `Ok` on
    /// the connection, and a second answer says how it ended.
    1 => fn stage / Stage (name: String, bytes: u64, allow_older: bool) -> Result<(), Vec<Issued>>;
    /// Stage the image in a file the host has (a removable volume
    /// mounted by Service Tools): copied whole, then verified.
    2 => fn stage_file / StageFile (path: String, allow_older: bool) -> Result<Staged, Vec<Issued>>;
    /// Throw away what is staged.
    3 => fn discard / Discard () -> Result<(), Vec<Issued>>;
    /// Install the verified image into the slot that is not running:
    /// the slot. Progress follows as events.
    4 => fn install / Install () -> Result<String, Vec<Issued>>;
    /// Restart into the pending slot; partitions running refuse it
    /// unless `shut_down`, which shuts each down through its system
    /// first, each given `timeout` seconds. `by` is who asked.
    5 => fn activate / Activate (shut_down: bool, timeout: u32, by: String) -> Result<(), Vec<Issued>>;
    /// The pending slot is pending no more.
    6 => fn cancel / Cancel () -> Result<(), Vec<Issued>>;
    /// Subscribe: the connection every event then arrives on.
    7 => fn subscribe / Subscribe () -> ();
}

/// A client: each call is its own connection.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct Client {
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl Client {
    /// A client of the socket at `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Subscribe, and call `f` with each event until it returns false or
    /// the connection ends; `ready` once the subscription is answered.
    ///
    /// # Errors
    /// The service is not there, or the connection broke.
    pub fn events(
        &self,
        ready: impl FnOnce(),
        mut f: impl FnMut(Event) -> bool,
    ) -> Result<(), ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(&Request::Subscribe {}))?;
        ipc::decode::<()>(&ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?)?;
        ready();
        loop {
            let frame = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
            if !f(ipc::decode(&frame)?) {
                return Ok(());
            }
        }
    }

    /// A stage's connection, its first answer read: the bytes are then
    /// written to it, and [`staged`] reads how it ended.
    ///
    /// # Errors
    /// The service is not there, or it refused.
    pub fn begin_stage(
        &self,
        name: &str,
        bytes: u64,
        allow_older: bool,
    ) -> Result<Result<std::os::unix::net::UnixStream, Vec<Issued>>, ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        let request = Request::Stage {
            name: name.to_string(),
            bytes,
            allow_older,
        };
        ipc::frame::write_frame(&mut stream, &ipc::encode(&request))?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        Ok(ipc::decode::<Result<(), Vec<Issued>>>(&answer)?.map(|()| stream))
    }
}

/// How a stage's connection ended: the image staged, or why not.
///
/// # Errors
/// The connection broke.
#[cfg(unix)]
pub fn staged(
    stream: &mut std::os::unix::net::UnixStream,
) -> Result<Result<Staged, Vec<Issued>>, ipc::WireError> {
    let answer = ipc::frame::read_frame_max(stream, MAX_FRAME)?;
    ipc::decode(&answer)
}

#[cfg(unix)]
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        // An install or a stage from a file writes and reads back whole
        // UKIs: it is given its time.
        let wait = Some(std::time::Duration::from_secs(300));
        stream.set_read_timeout(wait)?;
        stream.set_write_timeout(wait)?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(req))?;
        ipc::frame::read_frame_max(&mut stream, MAX_FRAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_socket_carries_round_trips() {
        let s = Status {
            slots: vec![Slot {
                name: "A".into(),
                release: "V1R0M0".into(),
                running: true,
                last: Attempt::Confirmed,
                ..Slot::default()
            }],
            staged: Some(Staged {
                name: "V1R0M1".into(),
                verification: Verification::Refused,
                error: vec![qvsmsg::NAME_INVALID.with(&["x"])],
                ..Staged::default()
            }),
            installing: None,
        };
        assert_eq!(ipc::decode::<Status>(&ipc::encode(&s)).unwrap(), s);
        let e = Event::Installing {
            progress: Progress {
                slot: "B".into(),
                percent: 50,
                time: 1,
            },
        };
        assert_eq!(ipc::decode::<Event>(&ipc::encode(&e)).unwrap(), e);
    }
}
