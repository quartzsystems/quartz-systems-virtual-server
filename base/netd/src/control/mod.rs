// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The network service's socket: one request and one answer per
//! connection, each an `ipc` frame, as PID 1's control socket.

#[cfg(unix)]
use std::path::{Path, PathBuf};

/// An interface, as status shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetStatus {
    /// Its role as `network.toml` writes it: empty for the first.
    pub role: String,
    /// The interface, empty when there is none.
    pub interface: String,
    /// The device it is on: the interface itself, or the link a VLAN
    /// interface is on; empty when there is none.
    pub device: String,
    /// Whether it has carrier.
    pub carrier: bool,
    /// `address/prefix`, empty when it has none.
    pub address: String,
    /// The default gateway, empty when there is none.
    pub gateway: String,
    /// Its status line: the status screen's management line for the
    /// first interface.
    pub line: String,
}
ipc::wire_struct!(NetStatus {
    role,
    interface,
    device,
    carrier,
    address,
    gateway,
    line
});

/// [`TrialStatus::state`]: no change was made.
pub const TRIAL_NONE: &str = "none";
/// [`TrialStatus::state`]: a change is on trial.
pub const TRIAL_TRYING: &str = "trying";
/// [`TrialStatus::state`]: the change was kept and written.
pub const TRIAL_KEPT: &str = "kept";
/// [`TrialStatus::state`]: the change was taken back.
pub const TRIAL_RESTORED: &str = "restored";

/// How a change made with [`Net::apply`] stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrialStatus {
    /// [`TRIAL_NONE`], [`TRIAL_TRYING`], [`TRIAL_KEPT`] or
    /// [`TRIAL_RESTORED`].
    pub state: String,
    /// While trying: the seconds until the change is taken back.
    pub seconds_left: u32,
    /// The status screen's management line, as it is now.
    pub line: String,
    /// Why a change was taken back: the status line of the interface
    /// that did not work.
    pub reason: String,
}
ipc::wire_struct!(TrialStatus {
    state,
    seconds_left,
    line,
    reason
});

ipc::rpc! {
    request Request, service Net, client Calls, dispatch dispatch;
    /// Read the state files again and apply what changed: an interface
    /// that did not change is left running.
    0 => fn reload / Reload () -> Result<(), String>;
    /// The first interface: the management interface.
    1 => fn status / Status () -> NetStatus;
    /// Put the network of `network` (the text of a `network.toml`) on
    /// the interfaces, on trial: it is written only once each interface
    /// it changes works, or it is confirmed, and taken back after thirty
    /// seconds otherwise. The interfaces it leaves alone run on.
    2 => fn apply / Apply (network: String) -> Result<(), String>;
    /// How the change made last stands.
    3 => fn trial / Trial () -> TrialStatus;
    /// Keep the change on trial whatever the interface says.
    4 => fn confirm / Confirm () -> Result<(), String>;
    /// Set the clock, and the RTC from it: seconds since the epoch.
    5 => fn set_clock / SetClock (secs: i64) -> Result<(), String>;
    /// Every interface, in `network.toml`'s order.
    6 => fn interfaces / Interfaces () -> Vec<NetStatus>;
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
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        supervise::control::exchange(&self.path, &ipc::encode(req))
    }
}
