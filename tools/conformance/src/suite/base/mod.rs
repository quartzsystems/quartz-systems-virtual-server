// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The base system's suites, on the test image. Each boots the image
//! under QEMU and asserts on the serial console and, through the test
//! hooks' channel, on the control socket's status.

mod inventory;
mod network;
pub mod power;
mod slot;
mod status;
mod supervision;
mod time;

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use refcode::RefCode;
use release::{fact, screen};

use crate::qemu::Launch;
use crate::qemu::channel::Channel;
use crate::qemu::session::Session;

/// A suite: what it saw, or why it did not pass.
pub type SuiteFn = fn(&Run<'_>) -> Result<String, String>;

/// The suites, in the order of the handoff.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("status", status::status),
    ("dhcp", network::dhcp),
    ("static-vlan", network::static_vlan),
    ("time", time::time),
    ("inventory", inventory::inventory),
    ("supervision", supervision::supervision),
    ("power-pull", power::power_pull),
    ("slot-confirm", slot::slot_confirm),
    ("shutdown", power::shutdown),
    ("sockets", network::sockets),
];

/// The MAC of the guest's NIC, when a suite gives it one.
pub const MAC: &str = "52:54:00:12:34:56";
/// What QEMU's user network gives a guest by DHCP.
pub const DHCP_ADDRESS: &str = "10.0.2.15/24";
/// Its gateway.
pub const DHCP_GATEWAY: &str = "10.0.2.2";
/// Its name server.
pub const DHCP_DNS: &str = "10.0.2.3";
/// The state directory on the guest.
pub const STATE_DIR: &str = "/data/state";

/// What a suite is run with.
pub struct Run<'a> {
    /// The suite's name.
    pub name: &'static str,
    /// The test disk image.
    pub disk: &'a Path,
    /// Where scratch files go.
    pub work: &'a Path,
    /// How guests are launched.
    pub base: &'a Launch,
    /// The built images.
    pub images: &'a crate::suite::Images,
    /// The release disk image, for the suites that boot it.
    pub release_disk: Option<&'a Path>,
    /// The disk was written by the installer: the host is configured,
    /// so what a suite would seed it gives the installer as answers.
    pub installed: bool,
    /// How long a boot may take.
    pub timeout: Duration,
    /// The serial logs of the suite's guests, for a failure.
    pub log: &'a RefCell<String>,
}

/// A running guest. When it goes, its serial log joins the suite's.
pub struct Guest<'a> {
    session: Session,
    log: &'a RefCell<String>,
}

impl Deref for Guest<'_> {
    type Target = Session;

    fn deref(&self) -> &Session {
        &self.session
    }
}

impl DerefMut for Guest<'_> {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

impl Drop for Guest<'_> {
    fn drop(&mut self) {
        self.session.kill();
        let mut log = self.log.borrow_mut();
        log.push_str(&self.session.log);
        log.push_str("\n----- the guest ended -----\n");
    }
}

impl<'a> Run<'a> {
    /// A scratch file of this suite.
    pub fn file(&self, name: &str) -> PathBuf {
        self.work.join(format!("{}-{name}", self.name))
    }

    /// The launch every suite starts from: the test image as a
    /// snapshot, no NIC, and the channel.
    pub fn launch(&self) -> Launch {
        Launch {
            disk: Some(self.disk.to_path_buf()),
            hook: Some(self.file("hook.sock")),
            ..self.base.clone()
        }
    }

    /// Start a guest.
    pub fn boot(&self, launch: &Launch) -> Result<Guest<'a>, String> {
        let session = Session::spawn(launch).map_err(|e| e.to_string())?;
        Ok(Guest {
            session,
            log: self.log,
        })
    }

    /// When a boot started now must be done.
    pub fn deadline(&self) -> Instant {
        Instant::now() + self.timeout
    }
}

/// The line a code is posted as, with what the posting adds.
pub fn posted(code: &RefCode, detail: &str) -> String {
    format!("{} {}", code.tag(), code.text_with(detail))
}

/// Wait for "Base ready". The seconds since the kernel started that
/// PID 1 posted with it.
pub fn base_ready(guest: &mut Session, deadline: Instant) -> Result<f64, String> {
    guest.expect(&format!("{}: ", posted(&refcode::BASE_READY, "")), deadline)?;
    let rest = guest.rest_of_line(deadline)?;
    rest.split_ascii_whitespace()
        .next()
        .and_then(|secs| secs.parse().ok())
        .ok_or_else(|| format!("`{rest}` does not start with the seconds since the kernel started"))
}

/// Boot to "Base ready" and connect the channel.
pub fn booted<'a>(run: &Run<'a>, launch: &Launch) -> Result<Guest<'a>, String> {
    let mut guest = run.boot(launch)?;
    let deadline = run.deadline();
    base_ready(&mut guest, deadline)?;
    guest.channel(deadline)?;
    Ok(guest)
}

/// The guest's channel, already connected by [`booted`].
pub fn channel(guest: &mut Session) -> Result<&mut Channel, String> {
    guest.channel(Instant::now() + Duration::from_secs(10))
}

/// Whether `log` has a whole status screen with every one of `rows`.
pub fn has_screen(log: &str, rows: &[String]) -> bool {
    log.split(screen::BEGIN).skip(1).any(|after| {
        after
            .split_once(screen::END)
            .is_some_and(|(block, _)| rows.iter().all(|row| block.contains(row.as_str())))
    })
}

/// Wait for a status screen on the serial port that has `rows`.
pub fn screen_with(guest: &mut Session, rows: &[String], deadline: Instant) -> Result<(), String> {
    let what = format!("a status screen with {rows:?}");
    guest.wait_for(&what, deadline, |log| has_screen(log, rows))
}

/// The value of a fact, or that the status lacks it.
pub fn fact_of<'s>(
    status: &'s supervise::control::HostStatus,
    key: &str,
) -> Result<&'s str, String> {
    status
        .fact(key)
        .ok_or_else(|| format!("the status has no `{key}`"))
}

/// Check that the host is up and whole: ready, and every service ready.
pub fn all_ready(status: &supervise::control::HostStatus) -> Result<(), String> {
    if fact_of(status, fact::READY)? != "yes" {
        return Err("the status says the base is not ready".into());
    }
    match status.services.iter().find(|s| s.state != "ready") {
        Some(s) => Err(format!("service {} is {}", s.name, s.state)),
        None => Ok(()),
    }
}

/// A seed for `seed.toml` with the tables given.
pub fn seed_toml(host: &str, network: &str) -> String {
    format!("schema_version = 1\n\n[host]\n{host}\n[network]\n{network}")
}
