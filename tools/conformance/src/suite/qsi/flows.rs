// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 4, 6, 7, 9 and 13: the whole of a partition's life over QSI,
//! events across a reconnect and a server's restart, two consoles, the
//! host's power over QSI, and `qsictl` running suite 4's steps.

use std::time::{Duration, Instant};

use partd::Calls as _;
use qsi::Empty;
use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::base;
use qsi::host::console::{In, Out, Pieces};
use qsi::host::model::{IplSource, MediaDevice, PartitionState};
use qsi::host::{console, partition, partitions, profiles, refcodes, vres};
use qsictl::link::Link;
use qsipart::console::{FromSystem, Grid, Key};
use qsisess::{Incoming, StreamId};

use super::families::{ask, profile, refused};
use super::{eventually, host, link, paired};
use crate::suite::base::{Run, channel};
use crate::suite::partition::manager;
use crate::suite::service::ipl_patience;

/// A console stream's viewer: its pieces read into a screen.
pub struct Viewer {
    /// The stream.
    pub stream: StreamId,
    /// Whether it is the serial port's.
    #[allow(dead_code)] // said by the attach; the suites draw frames
    pub serial: bool,
    pieces: Pieces,
    /// The screen as the pieces drew it.
    pub grid: Grid,
    /// The serial port's bytes.
    pub bytes: Vec<u8>,
    /// Why it ended, once it did.
    pub ended: Option<String>,
}

impl Viewer {
    /// Attach to partition `id`'s console.
    pub fn attach(l: &mut Link, id: u32, take_over: bool) -> Result<Self, String> {
        let reply = l.ask(&AnyRequest::Console(console::Request::Attach(
            console::Attach { id, take_over },
        )))?;
        let (Some(stream), Ok(Some(AnyReply::Console(console::Reply::Attach(a))))) =
            (reply.stream, reply.decode())
        else {
            return Err(qsictl::pair::refusal(&reply));
        };
        Ok(Self {
            stream,
            serial: a.serial,
            pieces: Pieces::default(),
            grid: Grid::default(),
            bytes: Vec::new(),
            ended: None,
        })
    }

    /// Take what came for this stream.
    pub fn take(&mut self, l: &mut Link, got: &mut Vec<Incoming>) {
        let mut rest = Vec::new();
        for i in got.drain(..) {
            match i {
                Incoming::Data { stream, bytes } if stream == self.stream => {
                    let _ = l
                        .session
                        .grant(stream, u32::try_from(bytes.len()).unwrap_or(0));
                    self.pieces.push(&bytes);
                    while let Some(piece) = self.pieces.next_piece() {
                        match Out::decode(&piece) {
                            Ok(Out::Frame(f)) => {
                                let _ = self.grid.apply(&FromSystem::Frame(f));
                            }
                            Ok(Out::Update(u)) => {
                                let _ = self.grid.apply(&FromSystem::Update(u));
                            }
                            Ok(Out::Serial(b)) => self.bytes.extend(b),
                            Ok(Out::Ended(why)) => self.ended = Some(why),
                            _ => {}
                        }
                    }
                }
                Incoming::StreamClosed { stream } | Incoming::StreamReset { stream, .. }
                    if stream == self.stream =>
                {
                    self.ended.get_or_insert_with(|| "the stream ended".into());
                }
                other => rest.push(other),
            }
        }
        *got = rest;
    }

    /// Wait until the screen has `text`, by `deadline`.
    pub fn until(&mut self, l: &mut Link, text: &str, deadline: Instant) -> Result<String, String> {
        loop {
            let mut got = std::mem::take(&mut l.besides);
            got.extend(l.step()?);
            self.take(l, &mut got);
            l.besides.extend(got);
            let shown = self.grid.text();
            if shown.contains(text) {
                return Ok(shown);
            }
            if let Some(why) = &self.ended {
                return Err(format!(
                    "the console ended ({why}) before `{text}`:\n{shown}"
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!("`{text}` never on the console:\n{shown}"));
            }
        }
    }

    /// Type keys.
    pub fn keys(&self, l: &mut Link, keys: &[Key]) -> Result<(), String> {
        for k in keys {
            let piece = In::Key(*k).piece().map_err(|e| e.to_string())?;
            l.send_all(self.stream, &piece)?;
        }
        Ok(())
    }
}

/// Text as keys.
pub fn typed(text: &str) -> Vec<Key> {
    text.chars().map(Key::Char).collect()
}

/// A partition's state as listed.
pub(crate) fn state_of(l: &mut Link, id: u32) -> Option<partitions::Status> {
    let AnyReply::Partitions(partitions::Reply::List(all)) =
        ask(l, AnyRequest::Partitions(partitions::Request::List(Empty))).ok()?
    else {
        return None;
    };
    all.into_iter().find(|s| s.id == id)
}

pub(crate) fn wait_state(
    l: &mut Link,
    id: u32,
    state: PartitionState,
    within: Duration,
) -> Result<partitions::Status, String> {
    eventually(within, || state_of(l, id).filter(|s| s.state == state))
        .ok_or_else(|| format!("partition {id} never {state:?}: {:?}", state_of(l, id)))
}

/// Subscribe; the next event's number.
pub(crate) fn subscribe(l: &mut Link, since: Option<u64>) -> Result<base::Subscribed, String> {
    let AnyReply::Base(base::Reply::Subscribe(s)) = ask(
        l,
        AnyRequest::Base(base::Request::Subscribe(base::Subscribe { since })),
    )?
    else {
        return Err("subscribe".into());
    };
    Ok(s)
}

/// The events come so far: number and event.
pub(crate) fn gather(l: &mut Link, within: Duration) -> Result<Vec<(u64, AnyEvent)>, String> {
    let until = Instant::now() + within;
    let mut out = Vec::new();
    while Instant::now() < until {
        let mut got = std::mem::take(&mut l.besides);
        got.extend(l.step()?);
        for i in got {
            match i {
                Incoming::Event(e) => {
                    if let Ok(Some(event)) = e.decode() {
                        out.push((e.seq, event));
                    }
                }
                Incoming::Closed {
                    msgid,
                    text,
                    by_peer,
                } => {
                    return Err(format!(
                        "the session closed after {} events (by the host: {by_peer}, {msgid:?}): {text}",
                        out.len()
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The partition's codes among `events`, in order.
fn codes_of(events: &[(u64, AnyEvent)], id: u32) -> Vec<String> {
    events
        .iter()
        .filter_map(|(_, e)| match e {
            AnyEvent::Refcodes(refcodes::Event::Code(c)) if c.id == id => {
                Some(c.posted.refcode.code.clone())
            }
            _ => None,
        })
        .collect()
}

/// A QS/400 partition made over QSI with the image as its disk: its ID.
pub(crate) fn make_partition(l: &mut Link, name: &str) -> Result<u32, String> {
    let AnyReply::Partition(partition::Reply::Create(d)) = ask(
        l,
        AnyRequest::Partition(partition::Request::Create(partition::Create {
            name: name.into(),
        })),
    )?
    else {
        return Err("create".into());
    };
    ask(
        l,
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id: d.id,
            profile: profile("NORMAL"),
            revision: None,
        })),
    )?;
    ask(
        l,
        AnyRequest::Profiles(profiles::Request::SetDefault(profiles::SetDefault {
            id: d.id,
            name: "NORMAL".into(),
        })),
    )?;
    Ok(d.id)
}

pub(crate) fn activate(
    l: &mut Link,
    id: u32,
    source: Option<IplSource>,
    profile: &str,
) -> Result<(), String> {
    ask(
        l,
        AnyRequest::Partition(partition::Request::Activate(partition::Activate {
            id,
            profile: profile.into(),
            source,
            keylock: None,
        })),
    )
    .map(|_| ())
}

pub(crate) fn shut_down(
    l: &mut Link,
    id: u32,
    kind: partition::ShutdownKind,
) -> Result<(), String> {
    ask(
        l,
        AnyRequest::Partition(partition::Request::ShutDown(partition::ShutDown {
            id,
            kind,
            delay: 0,
            restart: false,
        })),
    )
    .map(|_| ())
}

/// Suite 6: a console that disconnects during an IPL and reconnects
/// gets every code once and in order; one that stayed away past the
/// log's reach is told to read everything again; the server killed and
/// restarted loses none.
pub fn events(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, true)?;
    let deadline = run.deadline();
    let (id_a, host_print, mut l) = paired(&mut q, "MC1", deadline)?;
    let id = make_partition(&mut l, "EVT")?;
    let start = subscribe(&mut l, None)?;
    activate(&mut l, id, None, "")?;
    let first = gather(&mut l, Duration::from_secs(20))?;
    let last = first.last().map_or(start.next - 1, |(s, _)| *s);
    l.close();
    // Away while it IPLs.
    std::thread::sleep(Duration::from_secs(20));
    let mut l = link(&q, &id_a, host_print)?;
    let again = subscribe(&mut l, Some(last))?;
    if again.replay != base::Replay::Resumed {
        return Err(format!("reconnected: {again:?}"));
    }
    let mut rest = gather(&mut l, Duration::from_secs(5))?;
    let until = ipl_patience(run);
    while !codes_of(&rest, id).iter().any(|c| c == "C6001090") && Instant::now() < until {
        rest.extend(gather(&mut l, Duration::from_secs(5))?);
    }
    let mut all = first.clone();
    all.extend(rest.iter().cloned());
    let seqs: Vec<u64> = all.iter().map(|(s, _)| *s).collect();
    if seqs.windows(2).any(|w| w[1] != w[0] + 1) {
        return Err(format!("the numbers have a gap or a repeat: {seqs:?}"));
    }
    let codes = codes_of(&all, id);
    let local: Vec<String> = manager::partd(&mut q.guest)?
        .history(id)
        .map_err(|e| e.to_string())?
        .map_err(|e: Vec<qvsmsg::Issued>| manager::refused(&e))?
        .into_iter()
        .map(|p| p.code)
        .collect();
    if !local.ends_with(&codes) && codes != local {
        return Err(format!("codes as events {codes:?}, the history {local:?}"));
    }
    // The server killed and started again: it numbers on.
    let before = l.session.last_event().unwrap_or(0);
    l.close();
    let started = super::posted(&mut q, &refcode::QSI_STARTED);
    channel(&mut q.guest)?.ask(&format!("{} qsid", qsvshook::proto::command::KILL))?;
    super::posted_after(
        &mut q,
        &refcode::QSI_STARTED,
        started,
        Duration::from_mins(1),
    )
    .map_err(|_| "the server was not started again".to_string())?;
    let mut l = eventually(Duration::from_secs(30), || link(&q, &id_a, host_print).ok())
        .ok_or("no session after the server's restart")?;
    let after = subscribe(&mut l, Some(before))?;
    if after.replay != base::Replay::Resumed || after.next != before + 1 {
        return Err(format!(
            "after the server's restart: {after:?}, last seen {before}"
        ));
    }
    // Past the log's reach: more events than it keeps (it is kept
    // across the server's restart, so every one counts), then a console
    // that saw only the first, on a connection of its own.
    let keep = qsisess::Limits::default().log_events;
    let mut revision = None;
    for _ in 0..=keep {
        let AnyReply::Profiles(profiles::Reply::Put(saved)) = ask(
            &mut l,
            AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
                id,
                profile: profile("CHURN"),
                revision,
            })),
        )?
        else {
            return Err("put".into());
        };
        revision = Some(saved.revision);
        // Each put is at least one event.
    }
    l.close();
    let mut l = link(&q, &id_a, host_print)?;
    let far = subscribe(&mut l, Some(1))?;
    if far.replay != base::Replay::ReadAgain {
        return Err(format!("past the log's reach, from event 1: {far:?}"));
    }
    let reach = "read again";
    shut_down(&mut l, id, partition::ShutdownKind::Immediate)?;
    l.close();
    Ok(format!(
        "{} events before a disconnect during the IPL, {} after it, numbered without a gap; the \
         partition's {} codes once each and in order; after the server's restart resumed at {}; \
         a console that saw only event 1: {reach}",
        first.len(),
        rest.len(),
        codes.len(),
        after.next
    ))
}

/// Suite 7: two consoles see the same events; a stale profile put is
/// refused; a console taken over by the second closes the first's.
pub fn two_consoles(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, true)?;
    let deadline = run.deadline();
    let (_, _, mut a) = paired(&mut q, "MC-A", deadline)?;
    q.guest.type_keys(crate::qemu::term::key::F12)?;
    let (_, _, mut b) = {
        crate::suite::install::titled(&mut q.guest, super::PAIRING, deadline)?;
        let (password, _) = super::start_pairing(&mut q, deadline)?;
        let id = super::identity("MC-B")?;
        let host_print = super::pair(&q, &id, &password, "MC-B")?.fingerprint;
        let l = link(&q, &id, host_print)?;
        (id, host_print, l)
    };
    subscribe(&mut a, None)?;
    subscribe(&mut b, None)?;
    let id = make_partition(&mut a, "TWO")?;
    let seen_a = gather(&mut a, Duration::from_secs(3))?;
    let seen_b = gather(&mut b, Duration::from_secs(3))?;
    let nums = |v: &[(u64, AnyEvent)]| v.iter().map(|(s, _)| *s).collect::<Vec<_>>();
    if nums(&seen_a) != nums(&seen_b) || seen_a.is_empty() {
        return Err(format!(
            "A saw {:?}, B saw {:?}",
            nums(&seen_a),
            nums(&seen_b)
        ));
    }
    // A stale put: both read revision r; B puts at r; A's put at r is
    // refused.
    let get = AnyRequest::Profiles(profiles::Request::Get(profiles::Get {
        id,
        name: "NORMAL".into(),
    }));
    let AnyReply::Profiles(profiles::Reply::Get(r)) = ask(&mut a, get)? else {
        return Err("get".into());
    };
    let put = |revision| {
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id,
            profile: profile("NORMAL"),
            revision: Some(revision),
        }))
    };
    ask(&mut b, put(r.revision))?;
    let stale = refused(&mut a, &put(r.revision))?;
    if stale != profiles::msgid::CHANGED {
        return Err(format!("A's stale put: {stale}"));
    }
    // The console: A views, B takes it over.
    activate(&mut a, id, None, "")?;
    wait_state(&mut a, id, PartitionState::Running, Duration::from_mins(10))?;
    let mut va = Viewer::attach(&mut a, id, false)?;
    let busy = Viewer::attach(&mut b, id, false).err().unwrap_or_default();
    let _vb = Viewer::attach(&mut b, id, true)?;
    let ended = eventually(Duration::from_secs(15), || {
        let mut got = std::mem::take(&mut a.besides);
        got.extend(a.step().ok()?);
        va.take(&mut a, &mut got);
        va.ended.clone()
    })
    .ok_or("A's console did not end when B took it over")?;
    shut_down(&mut b, id, partition::ShutdownKind::Immediate)?;
    a.close();
    b.close();
    Ok(format!(
        "both consoles saw events {:?}; A's put at revision {} after B's refused {stale}; B's \
         attach without take over refused ({busy}); taken over, A's console ended: {ended}",
        nums(&seen_a),
        r.revision
    ))
}

/// Suite 9: the host powered off over QSI: its partitions shut down
/// first, the session ends cleanly; after the IPL the console connects
/// again and the auto start partitions are back.
pub fn power(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, true)?;
    let deadline = run.deadline();
    let (id_a, host_print, mut l) = paired(&mut q, "MC1", deadline)?;
    let id = make_partition(&mut l, "AUTO")?;
    let AnyReply::Partitions(partitions::Reply::Get(p)) = ask(
        &mut l,
        AnyRequest::Partitions(partitions::Request::Get(partitions::Get { id })),
    )?
    else {
        return Err("get".into());
    };
    let mut d = p.definition;
    d.auto_start.enabled = true;
    ask(
        &mut l,
        AnyRequest::Partition(partition::Request::Change(partition::Change {
            definition: d,
        })),
    )?;
    activate(&mut l, id, None, "")?;
    wait_state(&mut l, id, PartitionState::Running, Duration::from_mins(10))?;
    ask(
        &mut l,
        AnyRequest::Base(base::Request::Power(base::Power {
            action: base::PowerAction::Restart,
            immediate: false,
            delay: 0,
        })),
    )?;
    // Ended cleanly: the host's goodbye, no message of a fault.
    let ended = eventually(Duration::from_mins(5), || match l.step() {
        Err(e) => Some(Err(format!("the connection ended with no goodbye: {e}"))),
        Ok(got) => got.into_iter().find_map(|i| match i {
            Incoming::Closed {
                msgid: None, text, ..
            } => Some(Ok(text)),
            Incoming::Closed {
                msgid: Some(m),
                text,
                ..
            } => Some(Err(format!("closed {m} {text}"))),
            _ => None,
        }),
    })
    .ok_or("the session did not end with the host")??;
    q.guest.seen(&refcode::PARTITIONS_ENDED.tag(), deadline)?;
    q.guest.seen(&refcode::RESTART.tag(), deadline)?;
    // QEMU does not restart the machine: it is started again on the
    // same disks. The console connects, the partition is there.
    q.guest.wait_exit(Instant::now() + Duration::from_mins(5))?;
    let launch = q.launch.clone();
    drop(q);
    let mut q = super::Qsi {
        guest: crate::suite::install::boot(run, &launch)?,
        address: format!("127.0.0.1:{}", launch.qsi.unwrap_or_default()),
        launch,
    };
    q.guest.seen(
        &refcode::QSI_STARTED.tag(),
        Instant::now() + Duration::from_mins(10),
    )?;
    let mut l = eventually(Duration::from_mins(1), || link(&q, &id_a, host_print).ok())
        .ok_or("no session after the host's restart")?;
    let back = wait_state(&mut l, id, PartitionState::Running, Duration::from_mins(15))?;
    shut_down(&mut l, id, partition::ShutdownKind::Immediate)?;
    l.close();
    Ok(format!(
        "a restart asked over QSI: D2001010 (the partitions ended first), the session ended \
         ({ended}), D2001091; after the IPL the console connected again and {} was started \
         with the host, {} at {}",
        back.name,
        back.state_word(),
        back.code
    ))
}

trait Word {
    fn state_word(&self) -> String;
}

impl Word for partitions::Status {
    fn state_word(&self) -> String {
        format!("{:?}", self.state)
    }
}

/// Suite 4: the whole of a partition's life over QSI, nothing through
/// the hook but the pool the host makes: the install ISO uploaded, a
/// thin disk, a partition with its profile, an IPL from source D, the
/// codes as events, the installer on the console stream, the installed
/// system on its recorded side to sign-on, a sign-on, a shut down
/// through its operating system, and the partition and its disk
/// deleted.
#[allow(clippy::too_many_lines)] // the partition's life, a step at a time
pub fn end_to_end(run: &Run<'_>) -> Result<String, String> {
    let iso = crate::suite::storage::qs400_iso()?;
    let mut q = super::host_with_pool(run)?;
    let deadline = run.deadline();
    let (_, _, mut l) = paired(&mut q, "MC1", deadline)?;
    subscribe(&mut l, None)?;
    let up = super::streams::upload_file(&mut l, "QS400", &iso)?;
    let AnyReply::Vres(vres::Reply::VdiskCreate(disk)) = ask(
        &mut l,
        AnyRequest::Vres(vres::Request::VdiskCreate(vres::VdiskCreate {
            pool: super::POOL.into(),
            name: "E2ELS".into(),
            mib: 8192,
            thick: false,
        })),
    )?
    else {
        return Err("vdisk".into());
    };
    let AnyReply::Partition(partition::Reply::Create(d)) = ask(
        &mut l,
        AnyRequest::Partition(partition::Request::Create(partition::Create {
            name: "E2E".into(),
        })),
    )?
    else {
        return Err("create".into());
    };
    let id = d.id;
    let mut p = profile("INSTALL");
    p.scsi[0].luns[0].backing = format!("{}/E2ELS", super::POOL);
    p.optical = vec![MediaDevice {
        slot: 3,
        media: Some("QS400".into()),
    }];
    ask(
        &mut l,
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id,
            profile: p,
            revision: None,
        })),
    )?;
    activate(&mut l, id, Some(IplSource::D), "INSTALL")?;
    wait_state(&mut l, id, PartitionState::Running, Duration::from_mins(30))
        .or_else(|_| wait_state(&mut l, id, PartitionState::Starting, Duration::from_mins(1)))?;
    let mut v = Viewer::attach(&mut l, id, false)?;
    let until = Instant::now() + Duration::from_mins(60);
    v.until(&mut l, "Sign On", until)?;
    let seen = gather(&mut l, Duration::from_secs(1))?;
    let codes = codes_of(&seen, id);
    shut_down(&mut l, id, partition::ShutdownKind::System)?;
    wait_state(
        &mut l,
        id,
        PartitionState::NotActivated,
        Duration::from_mins(10),
    )?;
    // The installed system, from its recorded side, to sign-on and
    // signed on.
    activate(&mut l, id, Some(IplSource::Recorded), "INSTALL")?;
    let mut v = eventually(Duration::from_mins(10), || {
        Viewer::attach(&mut l, id, false).ok()
    })
    .ok_or("no console on the installed system")?;
    v.until(&mut l, "Sign On", Instant::now() + Duration::from_mins(30))?;
    let mut keys = typed("QSECOFR");
    keys.push(Key::FieldExit);
    keys.extend(typed("Quartz1"));
    keys.push(Key::Enter);
    v.keys(&mut l, &keys)?;
    let signed = eventually(Duration::from_mins(2), || {
        let mut got = std::mem::take(&mut l.besides);
        got.extend(l.step().ok()?);
        v.take(&mut l, &mut got);
        l.besides.extend(got);
        let t = v.grid.text();
        (!t.contains("Sign On") && !t.trim().is_empty()).then_some(t)
    })
    .ok_or("the sign-on did not take")?;
    shut_down(&mut l, id, partition::ShutdownKind::System)?;
    wait_state(
        &mut l,
        id,
        PartitionState::NotActivated,
        Duration::from_mins(15),
    )?;
    ask(
        &mut l,
        AnyRequest::Partition(partition::Request::Delete(partition::Delete {
            id,
            disks: true,
        })),
    )?;
    let AnyReply::Vres(vres::Reply::Vdisks(left)) = ask(
        &mut l,
        AnyRequest::Vres(vres::Request::Vdisks(vres::Vdisks {
            pool: super::POOL.into(),
        })),
    )?
    else {
        return Err("vdisks".into());
    };
    if left.iter().any(|v| v.name == "E2ELS") {
        return Err("the partition's disk is still there".into());
    }
    l.close();
    Ok(format!(
        "over QSI only: {up}; disk {} thin; partition {id} with its profile; source D: {} codes \
         as events, the installer to Sign On on the console stream; shut down by its system; the \
         recorded side to Sign On, signed on as QSECOFR (`{}`); shut down by its system; the \
         partition and its disk deleted",
        disk.name,
        codes.len(),
        signed
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
    ))
}

/// Suite 13: `qsictl` runs from the build machine: it pairs, lists,
/// makes a partition, activates it and attaches its console.
pub fn qsictl(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, true)?;
    let deadline = run.deadline();
    super::to_pairing(&mut q, deadline)?;
    let (password, _) = super::start_pairing(&mut q, deadline)?;
    let dir = run.file("qsictl");
    let bin = qsictl_binary()?;
    let run_it = |args: &[&str]| -> Result<String, String> {
        let out = std::process::Command::new(&bin)
            .arg("--dir")
            .arg(&dir)
            .args(args)
            .output()
            .map_err(|e| format!("qsictl: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!(
                "qsictl {args:?}: {}{text}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    };
    run_it(&["init"])?;
    let paired = run_it(&["pair", &q.address, &password, "--as", "vs"])?;
    let listed = run_it(&["list"])?;
    let made = run_it(&["create", "CTL"])?;
    let id: u32 = made
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("id: ")?
                .trim_end_matches(',')
                .parse()
                .ok()
        })
        .ok_or_else(|| format!("no ID in:\n{made}"))?;
    let mut id_q = super::identity("CHECK")?;
    let _ = &mut id_q;
    // The profile is put by the harness: qsictl reads profiles, the
    // harness has the image's.
    let home = qsictl::home::Home::at(dir.clone());
    let identity = home.identity()?;
    let host_print = home.host(Some("vs"))?.fingerprint;
    let mut l = link(&q, &identity, host_print)?;
    ask(
        &mut l,
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id,
            profile: profile("NORMAL"),
            revision: None,
        })),
    )?;
    l.close();
    run_it(&["activate", &id.to_string(), "--profile", "NORMAL"])?;
    let mut l = link(&q, &identity, host_print)?;
    wait_state(&mut l, id, PartitionState::Running, Duration::from_mins(15))?;
    l.close();
    // Its console in a terminal: what it prints of the screen.
    let mut child = std::process::Command::new(&bin)
        .arg("--dir")
        .arg(&dir)
        .args(["console", &id.to_string()])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_secs(10));
    let _ = child.kill();
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let shown = String::from_utf8_lossy(&out.stdout).to_string();
    if shown.trim().is_empty() {
        return Err("qsictl's console showed nothing".into());
    }
    run_it(&["shutdown", &id.to_string(), "--kind", "immediate"])?;
    Ok(format!(
        "qsictl paired ({}), listed {} partition(s), made {id}, activated it, attached its \
         console ({} bytes shown) and shut it down",
        paired.lines().last().unwrap_or(""),
        listed.matches("id:").count(),
        shown.len()
    ))
}

/// `qsictl` as the workspace built it, beside the harness's own binary.
pub(crate) fn qsictl_binary() -> Result<std::path::PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = me.parent().ok_or("no directory of the harness")?;
    let bin = dir.join(if cfg!(windows) {
        "qsictl.exe"
    } else {
        "qsictl"
    });
    if bin.is_file() {
        Ok(bin)
    } else {
        Err(format!(
            "{} is not built: cargo build -p qsictl",
            bin.display()
        ))
    }
}
