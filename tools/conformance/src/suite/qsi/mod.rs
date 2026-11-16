// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 8's suites: the QSI host server driven from outside the host,
//! over its management network, by QS/400's own client session (through
//! `qsictl`'s link), with the pairing password read off Service Tools'
//! screen. The hook is used only to look at the host and to compare
//! with what each service says locally.

// Two consoles, a link, a host: short names read best in these steps.
#![allow(clippy::many_single_char_names)]

mod families;
mod flows;
mod hostile;
mod pairing;
mod streams;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use poold::Calls as _;
use qsictl::link::Link;
use qsitrust::pair::Paired;
use qsitrust::{Fingerprint, Identity};

use super::base::{Guest, MAC, Run, SuiteFn, channel};
use super::install::{Kind, PASSWORD, USER, boot, installed, machine, option, sign_on};

/// Phase 8's suites (the handoff's 2 to 14; its 1 is every earlier
/// suite, its 15 the fuzz target, both the build tool's).
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("qsi-pairing", pairing::pairing),
    ("qsi-trust", pairing::trust),
    ("qsi-end-to-end", flows::end_to_end),
    ("qsi-families", families::families),
    ("qsi-events", flows::events),
    ("qsi-two-consoles", flows::two_consoles),
    ("qsi-streams", streams::streams),
    ("qsi-power", flows::power),
    ("qsi-service-kill", families::service_kill),
    ("qsi-hostile", hostile::hostile),
    ("qsi-listen", hostile::listen),
    ("qsi-qsictl", flows::qsictl),
    ("qsi-log", hostile::log),
];

/// The title of option 4.
pub const PAIRING: &str = "Management Console Pairing";

/// A host with its QSI port forwarded to the harness.
pub struct Qsi<'a> {
    /// The host.
    pub guest: Guest<'a>,
    /// Where the harness reaches its server: `127.0.0.1:port`.
    pub address: String,
}

/// A port nothing on the harness's side listens on.
pub fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| e.to_string())
}

/// An installed host on its management network, its server listening;
/// with the QS/400 image as a further disk when `partition`.
pub fn host<'a>(run: &Run<'a>, partition: bool) -> Result<Qsi<'a>, String> {
    host_disks(run, partition, &[])
}

/// As [`host`], with `more` disks after the partition's.
pub fn host_disks<'a>(run: &Run<'a>, partition: bool, more: &[PathBuf]) -> Result<Qsi<'a>, String> {
    let disks = installed(run, Kind::Single)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    let port = free_port()?;
    launch.nic = Some(MAC.into());
    launch.keys = true;
    launch.qsi = Some(port);
    if partition {
        let image = super::partition::qs400_image()?;
        let copy = run.file("unit.img");
        crate::qemu::image::sparse_copy(&image, &copy)?;
        launch.memory_mb = super::partition::HOST_MEMORY_MB;
        launch.cpus = super::partition::HOST_CPUS;
        launch.prealloc = true;
        launch.extra.push(crate::qemu::Drive {
            path: copy,
            serial: "QSVSUNIT0".into(),
            place: Some(4),
        });
    }
    for (i, disk) in more.iter().enumerate() {
        launch.extra.push(crate::qemu::Drive {
            path: disk.clone(),
            serial: format!("QSVSUNIT{}", i + 1),
            place: Some(5 + u8::try_from(i).unwrap_or(0)),
        });
    }
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();
    guest.seen(&refcode::QSI_STARTED.tag(), deadline)?;
    // What the partitions say would be drawn over Service Tools' screen.
    channel(&mut guest)?.ask(&format!(
        "{} {} {}",
        qsvshook::proto::command::PUT,
        qsvshook::proto::QUIET,
        hex::encode(b"1")
    ))?;
    Ok(Qsi {
        guest,
        address: format!("127.0.0.1:{port}"),
    })
}

/// The pool an end-to-end suite's host makes at the host, the media
/// repository.
pub const POOL: &str = "POOL1";

/// A host with a partition's disk and a blank pool disk, the pool made
/// at the host (QSI does not make pools) and designated the media
/// repository.
pub fn host_with_pool<'a>(run: &Run<'a>) -> Result<Qsi<'a>, String> {
    let pool = run.file("pool0.img");
    crate::qemu::image::blank(
        &pool,
        4 * crate::suite::storage::POOL_DISK_BYTES + (2 << 30),
    )?;
    let mut q = host_disks(run, true, &[pool])?;
    let names = crate::suite::storage::disk_names(&mut q.guest, 2)?;
    crate::suite::storage::said(crate::suite::storage::poold(&mut q.guest)?.create_pool(
        POOL.into(),
        "none".into(),
        vec![names[1].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    crate::suite::storage::pool_until(&mut q.guest, POOL, run.deadline(), |p| !p.mount.is_empty())?;
    crate::suite::storage::said(
        crate::suite::storage::poold(&mut q.guest)?.designate(POOL.into()),
    )?;
    Ok(q)
}

/// The password on the `Start Pairing` screen: its four groups of four.
pub fn password_of(screen: &str) -> Option<String> {
    screen.lines().find_map(|l| {
        let words: Vec<&str> = l.split_whitespace().collect();
        (words.len() == 4
            && words
                .iter()
                .all(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_alphanumeric())))
        .then(|| words.concat())
    })
}

/// The screen with the password, the fingerprint and the times masked,
/// for its golden file.
pub fn masked(screen: &str) -> String {
    screen
        .lines()
        .map(|l| {
            let words: Vec<&str> = l.split_whitespace().collect();
            let hexish = !words.is_empty()
                && words
                    .iter()
                    .all(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_alphanumeric()));
            if hexish {
                l.chars()
                    .map(|c| if c.is_ascii_alphanumeric() { 'X' } else { c })
                    .collect()
            } else if l.contains("Expires") {
                l.chars()
                    .map(|c| if c.is_ascii_digit() { '9' } else { c })
                    .collect()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sign on to Service Tools at the serial port and go to option 4.
pub fn to_pairing(q: &mut Qsi<'_>, deadline: Instant) -> Result<String, String> {
    sign_on(&mut q.guest, USER, PASSWORD, deadline)?;
    option(&mut q.guest, 4, PAIRING, deadline)
}

/// From option 4's menu: start pairing and read the password, then F12
/// back to the menu unless `stay`.
pub fn start_pairing(q: &mut Qsi<'_>, deadline: Instant) -> Result<(String, String), String> {
    let screen = option(&mut q.guest, 2, "Start Pairing", deadline)?;
    let password = password_of(&screen).ok_or_else(|| format!("no password on:\n{screen}"))?;
    Ok((password, screen))
}

/// A console's identity, made now.
pub fn identity(name: &str) -> Result<Identity, String> {
    let now = appliance::time::now_secs();
    Identity::make(name, "", now).map_err(|e| e.to_string())
}

/// Pair as `id` with `password`.
pub fn pair(q: &Qsi<'_>, id: &Identity, password: &str, name: &str) -> Result<Paired, String> {
    qsictl::pair::pair(&q.address, id, password, name)
}

/// A pinned console's link.
pub fn link(q: &Qsi<'_>, id: &Identity, host: Fingerprint) -> Result<Link, String> {
    Link::connect(&q.address, id, Some(host))
}

/// Pair a console named `name` from Service Tools' password: its
/// identity, the host's certificate, and a link with a session.
pub fn paired(
    q: &mut Qsi<'_>,
    name: &str,
    deadline: Instant,
) -> Result<(Identity, Fingerprint, Link), String> {
    to_pairing(q, deadline)?;
    let (password, _) = start_pairing(q, deadline)?;
    let id = identity(name)?;
    let host = pair(q, &id, &password, name)?.fingerprint;
    let l = link(q, &id, host)?;
    Ok((id, host, l))
}

/// Wait until `f` holds, by `within`.
pub fn eventually<T>(within: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let until = Instant::now() + within;
    loop {
        if let Some(t) = f() {
            return Some(t);
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A file of the suite's.
pub fn scratch(run: &Run<'_>, name: &str) -> PathBuf {
    run.file(name)
}
