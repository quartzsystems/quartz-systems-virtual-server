// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 5's suites: storage pools, virtual disks, the media library,
//! optical and tape. QEMU gives the host further disks for its pools,
//! and the pool service is asked through the test hooks' channel, as
//! the partition manager is: the requests `docs/POOLD.md` documents.
//!
//! - [`pools`]: pools made, restarted, their state file made again,
//!   units pulled and lost, the service killed.
//! - [`vdisks`]: thin and thick, a full pool, who holds a disk.
//! - [`media`]: the library, uploads, damaged media for IPL source D.
//! - [`install`]: QS/400 installed from its ISO into a virtual disk.
//! - [`devices`]: the optical and tape devices, through the probe.
//! - [`tools`]: Service Tools making a pool, against golden files.

mod devices;
mod install;
mod media;
mod pools;
mod tools;
mod vdisks;

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use poold::proto::{Calls, PoolStatus, Request};
use qsvshook::proto::{command, word};

use super::base::{Guest, Run, SuiteFn};
use crate::qemu::channel::Channel;
use crate::qemu::image::blank;
use crate::qemu::session::Session;

/// The suites, in the order of the handoff.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("pool-create", pools::create),
    ("pool-install", install::install),
    ("pool-thin-thick", vdisks::thin_thick),
    ("pool-full", vdisks::full),
    ("pool-pulled-mirrored", pools::pulled_mirrored),
    ("pool-pulled-raid5", pools::pulled_raid5),
    ("pool-pulled-raid6", pools::pulled_raid6),
    ("pool-lost", pools::lost),
    ("pool-power-pull", pools::power_pull),
    ("pool-kill", pools::kill),
    ("pool-vdisk-rules", vdisks::rules),
    ("pool-optical", devices::optical),
    ("pool-tape", devices::tape),
    ("pool-media-damaged", media::damaged),
    ("pool-upload", media::upload_suite),
    ("pool-foreign", pools::foreign),
    ("pool-tools", tools::tools),
];

pub use install::qs400_iso;

/// The environment variable that names QS/400's installation ISO.
pub const QS400_ISO_ENV: &str = "QSVS_QS400_ISO";

/// A pool disk's size: room for a small pool and its record.
pub const POOL_DISK_BYTES: u64 = 2 << 30;

/// The pool service behind a channel.
pub struct Poold<'a> {
    channel: &'a mut Channel,
}

impl Calls for Poold<'_> {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        let line = format!("{} {}", command::POOLD, hex::encode(ipc::encode(req)));
        let lines = self
            .channel
            .ask_within(&line, Duration::from_secs(120))
            .map_err(io::Error::other)?;
        let answer = lines
            .first()
            .and_then(|l| l.strip_prefix(word::HEX))
            .ok_or_else(|| io::Error::other(format!("`{}` answered {lines:?}", req.name())))?;
        Ok(hex::decode(answer.trim()).map_err(io::Error::other)?)
    }
}

/// The pool service of the host `guest` is.
pub fn poold(guest: &mut Session) -> Result<Poold<'_>, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    Ok(Poold {
        channel: guest.channel(deadline)?,
    })
}

/// What went wrong with a call: the channel, or the messages.
pub fn said<T>(
    result: Result<Result<T, Vec<qvsmsg::Issued>>, ipc::WireError>,
) -> Result<T, String> {
    result
        .map_err(|e| format!("the pool service: {e}"))?
        .map_err(|errors| super::partition::manager::refused(&errors))
}

/// `count` blank pool disks for this suite, of `bytes` each.
pub fn pool_disks(run: &Run<'_>, count: usize, bytes: u64) -> Result<Vec<PathBuf>, String> {
    (0..count)
        .map(|i| {
            let path = run.file(&format!("pool{i}.img"));
            blank(&path, bytes)?;
            Ok(path)
        })
        .collect()
}

/// The kernel names the host gives the disks of serials `QSVSUNIT<i>`,
/// in order.
pub fn disk_names(guest: &mut Session, count: usize) -> Result<Vec<String>, String> {
    let disks = poold(guest)?
        .disks()
        .map_err(|e| format!("the pool service: {e}"))?;
    (0..count)
        .map(|i| {
            let serial = format!("QSVSUNIT{i}");
            disks
                .iter()
                .find(|d| d.serial == serial)
                .map(|d| d.name.clone())
                .ok_or_else(|| format!("no disk has the serial {serial}: {disks:?}"))
        })
        .collect()
}

/// The pool `name`, by `deadline` in a state `ok` takes.
pub fn pool_until(
    guest: &mut Session,
    name: &str,
    deadline: Instant,
    ok: impl Fn(&PoolStatus) -> bool,
) -> Result<PoolStatus, String> {
    loop {
        let pools = poold(guest)?
            .pools()
            .map_err(|e| format!("the pool service: {e}"))?;
        let p = pools.iter().find(|p| p.name == name).cloned();
        match p {
            Some(p) if ok(&p) => return Ok(p),
            p if Instant::now() >= deadline => {
                return Err(format!("pool {name} is {p:?}"));
            }
            _ => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// Boot the host with its further disks `disks` and wait until both
/// its pool service and its partition manager answer.
pub fn host<'a>(run: &Run<'a>, disks: &[PathBuf]) -> Result<Guest<'a>, String> {
    let launch = super::partition::launch(run, disks)?;
    let mut guest = super::partition::host(run, &launch)?;
    let deadline = run.deadline();
    loop {
        match poold(&mut guest)?.pools() {
            Ok(_) => return Ok(guest),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!("the pool service does not answer: {e}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}
