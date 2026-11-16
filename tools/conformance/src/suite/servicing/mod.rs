// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 9's suites: problems, the host's dump, code update, Service
//! Tools option 8, a host's configuration captured and put back, a
//! processor without what QSI's cryptography needs, and `qsictl`. Each
//! drives an installed host over QSI from outside, as Phase 8's do; the
//! hook only looks at the host or takes a disk from under it.
//!
//! An update needs the image of another release: `QSVS_UPDATE_IMAGE` is
//! a test image's update of `V1R0M1`, built beside this one with the
//! same signing key (`docs/PHASE9.md`). The test image's own update
//! image is the build's.

// A host, a link, a console: short names read best in these steps.
#![allow(clippy::many_single_char_names)]

mod capture;
mod dump;
mod problems;
mod processor;
mod tools;
mod update;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::{Empty, base};
use qsictl::link::Link;
use qsitrust::{Fingerprint, Identity};

use super::base::{MAC, Run, SuiteFn, channel};
use super::install::{Kind, boot, installed, machine};
use super::qsi::families::ask;
use super::qsi::{Qsi, eventually, free_port};
use crate::qemu::{Drive, Launch, firmware};

/// Phase 9's suites (the handoff's 1 to 15; its 16, the fuzz targets,
/// is the build tool's).
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("svc-away", problems::away),
    ("svc-problem-pool", problems::pool),
    ("svc-problems", problems::conditions),
    ("svc-problem-kill", problems::kill),
    ("svc-hostdump", dump::host_dump),
    ("svc-update", update::good),
    ("svc-update-rollback", update::rollback),
    ("svc-update-refused", update::refused),
    ("svc-update-power", update::power),
    ("svc-update-mirror", update::mirror),
    ("svc-update-back", update::back),
    ("svc-tools", tools::tools),
    ("svc-capture", capture::capture),
    ("svc-processor", processor::processor),
    ("svc-qsictl", update::qsictl),
];

/// The update image of another release: `QSVS_UPDATE_IMAGE`.
pub const UPDATE_ENV: &str = "QSVS_UPDATE_IMAGE";

/// What a suite's host is made with.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// Two load-source disks, mirrored.
    pub mirrored: bool,
    /// The QS/400 image as a further disk, for a partition.
    pub partition: bool,
    /// Blank pool disks after it.
    pub pool_disks: usize,
    /// The test hooks the host is started with (`refuse-confirm`).
    pub hooks: Vec<&'static str>,
    /// The processor in place of the host's.
    pub cpu: Option<String>,
    /// A removable volume.
    pub usb: Option<PathBuf>,
    /// A partition's memory, so the host reserves huge pages, as with
    /// `partition`, without the image.
    pub memory: bool,
}

/// A suite's host: on its management network, with its own firmware
/// variables (so `BootNext` and `BootOrder` last across its restarts),
/// started again on the same disks after each.
pub struct Rig<'a> {
    /// The host and how it was started.
    pub q: Qsi<'a>,
    /// Its load-source disks.
    pub disks: Vec<PathBuf>,
    /// Its pool disks.
    pub pool_disks: Vec<PathBuf>,
}

/// An installed host as `opts` say, its server listening.
pub fn host<'a>(run: &Run<'a>, opts: &Opts) -> Result<Rig<'a>, String> {
    let kind = if opts.mirrored {
        Kind::Mirrored
    } else {
        Kind::Single
    };
    let disks = installed(run, kind)?;
    let places: Vec<Option<&std::path::Path>> = disks.iter().map(|d| Some(d.as_path())).collect();
    let mut launch = machine(run, &places);
    let template = firmware::locate_vars(&run.base.firmware).map_err(|e| e.to_string())?;
    let vars = run.file("vars.fd");
    std::fs::copy(&template, &vars).map_err(|e| format!("{}: {e}", vars.display()))?;
    launch.vars = Some(vars);
    launch.nic = Some(MAC.into());
    launch.keys = true;
    launch.qsi = Some(free_port()?);
    launch.monitor = Some(run.socket("monitor.sock"));
    launch.oem = opts.hooks.iter().map(|h| qsvshook::oem_string(h)).collect();
    launch.cpu.clone_from(&opts.cpu);
    launch.usb.clone_from(&opts.usb);
    if opts.memory {
        launch.memory_mb = super::partition::HOST_MEMORY_MB;
    }
    if opts.partition {
        let image = super::partition::qs400_image()?;
        let copy = run.file("unit.img");
        crate::qemu::image::sparse_copy(&image, &copy)?;
        launch.memory_mb = super::partition::HOST_MEMORY_MB;
        launch.cpus = super::partition::HOST_CPUS;
        launch.prealloc = true;
        launch.extra.push(Drive {
            path: copy,
            serial: "QSVSUNIT0".into(),
            place: Some(4),
        });
    }
    let mut pool_disks = Vec::new();
    for i in 0..opts.pool_disks {
        let path = run.file(&format!("pool{i}.img"));
        crate::qemu::image::blank(&path, super::storage::POOL_DISK_BYTES)?;
        launch.extra.push(Drive {
            path: path.clone(),
            serial: format!("QSVSUNIT{}", i + usize::from(opts.partition)),
            place: Some(5 + u8::try_from(i).unwrap_or(0)),
        });
        pool_disks.push(path);
    }
    let q = start(run, launch)?;
    Ok(Rig {
        q,
        disks,
        pool_disks,
    })
}

/// Start the host of `launch` and wait for its server.
pub fn start<'a>(run: &Run<'a>, launch: Launch) -> Result<Qsi<'a>, String> {
    // A host whose flaky service fails is never all ready: its server
    // is what is waited for.
    let flaky = launch
        .oem
        .contains(&qsvshook::oem_string(qsvshook::FLAKY_FAILS));
    let mut guest = if flaky {
        let mut g = run.boot(&launch)?;
        g.seen(&refcode::QSI_STARTED.tag(), run.deadline())?;
        g.channel(run.deadline())?;
        g
    } else {
        let mut g = boot(run, &launch)?;
        g.seen(&refcode::QSI_STARTED.tag(), run.deadline())?;
        g
    };
    channel(&mut guest)?.ask(&format!(
        "{} {} {}",
        qsvshook::proto::command::PUT,
        qsvshook::proto::QUIET,
        hex::encode(b"1")
    ))?;
    let port = launch.qsi.unwrap_or(0);
    Ok(Qsi {
        guest,
        address: format!("127.0.0.1:{port}"),
        launch,
    })
}

impl<'a> Rig<'a> {
    /// The host restarted by itself (its own restart ends QEMU): wait
    /// for that, then start it again on the same disks.
    pub fn restarted(&mut self, run: &Run<'a>, within: Duration) -> Result<(), String> {
        self.q.guest.wait_exit(Instant::now() + within)?;
        self.again(run)
    }

    /// Start the host again on the same disks.
    pub fn again(&mut self, run: &Run<'a>) -> Result<(), String> {
        let launch = self.q.launch.clone();
        // The old guest's log joins the suite's when it goes.
        let fresh = start(run, launch)?;
        self.q = fresh;
        Ok(())
    }

    /// Cut its power and start it again.
    pub fn power_cut(&mut self, run: &Run<'a>) -> Result<(), String> {
        self.q.guest.kill();
        self.again(run)
    }
}

/// A console paired with the host from Service Tools' password.
pub struct Console {
    /// Its identity.
    pub id: Identity,
    /// The host's certificate.
    pub host: Fingerprint,
    /// Its link.
    pub link: Link,
}

/// Pair a console named `name`.
pub fn console(q: &mut Qsi<'_>, name: &str, deadline: Instant) -> Result<Console, String> {
    let (id, host, link) = super::qsi::paired(q, name, deadline)?;
    // Signed off, so the serial port is free and the next sign-on finds
    // its prompt.
    sign_off(&mut q.guest)?;
    Ok(Console { id, host, link })
}

/// F3 from wherever Service Tools is at the serial port until it is
/// signed off and PID 1 asks for Enter again.
pub fn sign_off(g: &mut crate::suite::base::Guest<'_>) -> Result<(), String> {
    for _ in 0..6 {
        g.type_keys(crate::qemu::term::key::F3)?;
        let soon = Instant::now() + Duration::from_secs(3);
        if g.screen("signed off", soon, |s| s.contains("Press Enter to sign on"))
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("Service Tools did not sign off at the serial port".into())
}

impl Console {
    /// A new link, once the host is back after a restart.
    pub fn again(&mut self, q: &Qsi<'_>, within: Duration) -> Result<(), String> {
        let link = eventually(within, || super::qsi::link(q, &self.id, self.host).ok())
            .ok_or("no session after the restart")?;
        self.link.close();
        self.link = link;
        Ok(())
    }

    /// Ask, and the reply; a refusal is an error with its messages.
    pub fn ask(&mut self, r: AnyRequest) -> Result<AnyReply, String> {
        ask(&mut self.link, r)
    }

    /// Ask, expecting a refusal: every identifier of it, the first first.
    pub fn refused(&mut self, r: &AnyRequest) -> Result<Vec<String>, String> {
        let reply = self.link.ask(r)?;
        let Some((first, _)) = reply.refusal() else {
            return Err(format!("{r:?} was not refused"));
        };
        let text = qsictl::pair::refusal(&reply);
        let mut ids = vec![first.to_string()];
        for w in text.split_whitespace() {
            let w = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if (w.starts_with("QSI") || w.starts_with("QVS"))
                && w.len() == 7
                && !ids.iter().any(|i| i == w)
            {
                ids.push(w.to_string());
            }
        }
        Ok(ids)
    }

    /// Subscribe from `since`.
    pub fn subscribe(&mut self, since: Option<u64>) -> Result<base::Subscribed, String> {
        super::qsi::flows::subscribe(&mut self.link, since)
    }

    /// The events until `done` finds what it waits for, within `within`:
    /// what it found.
    pub fn until<T>(
        &mut self,
        within: Duration,
        mut done: impl FnMut(&AnyEvent) -> Option<T>,
    ) -> Result<T, String> {
        let t0 = Instant::now();
        let mut seen = Vec::new();
        while t0.elapsed() < within {
            for (_, e) in super::qsi::flows::gather(&mut self.link, Duration::from_millis(500))? {
                if let Some(found) = done(&e) {
                    return Ok(found);
                }
                seen.push(format!("{e:?}").chars().take(120).collect::<String>());
            }
        }
        Err(format!("not seen in {within:?}; the events were {seen:?}"))
    }

    /// Who the host says it is.
    pub fn identify(&mut self) -> Result<base::Identity, String> {
        match self.ask(AnyRequest::Base(base::Request::Identify(Empty)))? {
            AnyReply::Base(base::Reply::Identify(i)) => Ok(i),
            other => Err(format!("{other:?}")),
        }
    }
}

/// The update image of another release, `QSVS_UPDATE_IMAGE`.
pub fn other_release() -> Result<PathBuf, String> {
    std::env::var_os(UPDATE_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            format!("{UPDATE_ENV} names no update image of another release (docs/PHASE9.md)")
        })
}

/// Kill the service `name`, waiting for it to be started again first
/// when a kill just before left it with no process.
pub fn kill(g: &mut crate::suite::base::Guest<'_>, name: &str) -> Result<(), String> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        match channel(g)?.ask(&format!("{} {name}", qsvshook::proto::command::KILL)) {
            Ok(_) => return Ok(()),
            Err(e) if e.contains("no process") && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}

/// The test image's own update image.
pub fn this_release(run: &Run<'_>) -> Result<PathBuf, String> {
    run.images
        .test_update
        .clone()
        .filter(|p| p.is_file())
        .ok_or_else(|| {
            "the test image's update image is not built: `build update --test-image`".into()
        })
}

/// The key the test image's manifests are signed with.
pub fn signing_key(run: &Run<'_>) -> Result<ed25519_dalek::SigningKey, String> {
    let path = run.images.test_key.clone().ok_or("no signing key")?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = hex::decode(text.trim()).map_err(|e| e.to_string())?;
    let seed: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "a key of 32 bytes".to_string())?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

/// Each state file's path and its length and sum.
pub type Sums = Vec<(String, (u64, u64))>;

/// Each state file of the host and its sum: what "the state exactly as
/// it was" compares.
pub fn state_sums(q: &mut Qsi<'_>) -> Result<Sums, String> {
    fn walk(
        c: &mut crate::qemu::channel::Channel,
        dir: &str,
        out: &mut Sums,
        depth: usize,
    ) -> Result<(), String> {
        for name in c.ls(dir)? {
            let path = format!("{dir}/{}", name.trim_end_matches('/'));
            if name.ends_with('/') {
                if depth > 0 {
                    walk(c, &path, out, depth - 1)?;
                }
            } else {
                out.push((path.clone(), c.sum(&path)?));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(channel(&mut q.guest)?, super::base::STATE_DIR, &mut out, 3)?;
    out.sort();
    Ok(out)
}

/// The problems a console reads: each one's code, status and resources.
pub fn problems(c: &mut Console) -> Result<Vec<qsi::service::Summary>, String> {
    let r = c.ask(AnyRequest::Service(qsi::service::Request::List(
        qsi::service::List {
            filter: qsi::service::Filter::default(),
            before: None,
            limit: 500,
        },
    )))?;
    match r {
        AnyReply::Service(qsi::service::Reply::List(p)) => Ok(p.problems),
        other => Err(format!("{other:?}")),
    }
}

/// One problem whole.
pub fn problem(c: &mut Console, number: u64) -> Result<qsi::service::Problem, String> {
    match c.ask(AnyRequest::Service(qsi::service::Request::Get(
        qsi::service::Get { number },
    )))? {
        AnyReply::Service(qsi::service::Reply::Get(p)) => Ok(p),
        other => Err(format!("{other:?}")),
    }
}

/// The open problems of `code`, by `within`, at least `count` of them.
pub fn problems_of(
    c: &mut Console,
    code: &str,
    count: usize,
    within: Duration,
) -> Result<Vec<qsi::service::Summary>, String> {
    let mut last = Vec::new();
    let found = eventually(within, || {
        last = problems(c).ok()?;
        let of: Vec<_> = last
            .iter()
            .filter(|s| s.refcode.code == code)
            .cloned()
            .collect();
        (of.len() >= count).then_some(of)
    });
    found.ok_or_else(|| {
        let all: Vec<String> = last
            .iter()
            .map(|s| format!("{} {}", s.number, s.refcode.code))
            .collect();
        format!("no {count} problem(s) of {code}: {all:?}")
    })
}
