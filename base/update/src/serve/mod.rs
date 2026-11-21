// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The update service on Linux (`docs/UPDATE.md`). It keeps what is
//! staged and what each slot holds in its directory on `/data`, writes
//! a slot's UKI to every load-source disk's ESP, and asks PID 1, which
//! owns the A/B record and the firmware's variables, to mark the slot
//! pending, cancel it, or activate it. Each step posts its code and
//! tells the subscribers.

mod steps;

use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use msg::Issued;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use serde::{Deserialize, Serialize};
use supervise::control::About;
use supervise::control::Calls as _;
use supervise::signal::{Signals, TERM};
use vrm::Release;

use crate::proto::{
    self, Attempt, Event, ImageInfo, Progress, Request, Slot, Staged, Status, Updates, Verification,
};
use crate::verify::{self, Refusal, Running};

/// What the service is given by the product it runs as.
#[derive(Clone)]
pub struct Host {
    /// The product's id (`appliance.id`): the socket's name.
    pub id: &'static str,
    /// Its images' format.
    pub format: &'static crate::Format,
    /// The messages it refuses with.
    pub messages: &'static crate::Messages,
    /// Post the product's code for one of the service's, with what it is
    /// about.
    pub post: fn(crate::Code, &str, &[About]),
    /// Share the socket with the QSI server's group.
    pub share: fn(&Path) -> io::Result<()>,
    /// The run directory: the socket.
    pub run_dir: &'static str,
    /// Its own directory on `/data`.
    pub dir: PathBuf,
    /// The state files, whose versions an image must read.
    pub state_dir: PathBuf,
    /// The trusted signers' public keys.
    pub keys: PathBuf,
    /// The product's name, as an image's manifest says it.
    pub product: &'static str,
    /// The release that runs.
    pub release: Release,
    /// Its build.
    pub build: &'static str,
    /// PID 1's control socket.
    pub control: PathBuf,
    /// What must be stopped before a restart.
    pub partitions: fn() -> Box<dyn crate::Partitions + Send>,
    /// What a load-source disk carries.
    pub names: &'static appliance::LoadSourceNames,
}

/// The image staged.
const IMAGE: &str = "staged.img";
/// What it is.
const STAGED: &str = "staged.json";
/// What each slot holds.
const SLOTS: &str = "slots.json";

/// What a slot holds, as the service keeps it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// Its release; empty when not known.
    pub release: String,
    /// Its build.
    pub build: String,
    /// Its fixes.
    pub fixes: Vec<String>,
    /// How its last activation ended.
    pub last: Attempt,
    /// Why it rolled back.
    #[serde(default)]
    pub reason: String,
    /// When it last changed.
    pub time: u64,
}

/// Slot A's and slot B's.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Kept {
    a: Held,
    b: Held,
}

impl Kept {
    pub(crate) fn of(&mut self, slot: &str) -> &mut Held {
        if slot == "A" {
            &mut self.a
        } else {
            &mut self.b
        }
    }
}

/// What is staged, kept beside its image once it verified.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct StagedKept {
    name: String,
    bytes: u64,
    image: ImageInfo,
    allow_older: bool,
    time: u64,
}

pub(crate) struct Core {
    pub(crate) host: Host,
    pub(crate) staged: Option<Staged>,
    allow_older: bool,
    /// An install or an activation under way, in words.
    pub(crate) busy: Option<String>,
    pub(crate) installing: Option<Progress>,
    subscribers: Vec<mpsc::Sender<Event>>,
}

pub(crate) type Shared = Arc<Mutex<Core>>;

pub(crate) fn lock(core: &Shared) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The product's messages, given once at the start.
static MESSAGES: std::sync::OnceLock<&'static crate::Messages> = std::sync::OnceLock::new();

/// The product's messages.
pub(crate) fn msgs() -> &'static crate::Messages {
    MESSAGES
        .get()
        .copied()
        .expect("the update service's messages are given at its start")
}

pub(crate) fn now() -> u64 {
    u64::try_from(appliance::time::now_secs()).unwrap_or(0)
}

impl Core {
    pub(crate) fn publish(&mut self, e: &Event) {
        self.subscribers.retain(|tx| tx.send(e.clone()).is_ok());
    }

    pub(crate) fn post(&self, code: crate::Code, detail: &str, about: &[About]) {
        (self.host.post)(code, detail, about);
    }

    pub(crate) fn pid1(&self) -> supervise::control::Client {
        supervise::control::Client::new(&self.host.control)
    }

    pub(crate) fn path(&self, file: &str) -> PathBuf {
        self.host.dir.join(file)
    }

    /// What runs, for verification.
    pub(crate) fn running(&self) -> Running {
        Running {
            format: self.host.format,
            product: self.host.product.to_string(),
            release: self.host.release,
            keys: fs::read(&self.host.keys).unwrap_or_default(),
            state: verify::state_versions(&self.host.state_dir),
        }
    }

    pub(crate) fn kept(&self) -> Kept {
        fs::read(self.path(SLOTS))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub(crate) fn keep(&self, kept: &Kept) {
        let written = serde_json::to_vec(kept)
            .map_err(io::Error::other)
            .and_then(|b| state::store::atomic_write(&self.path(SLOTS), &b));
        if let Err(e) = written {
            eprintln!("{SLOTS}: {e}");
        }
    }

    /// What PID 1 and the slots' record say together, settled: a slot
    /// that confirmed or came back, said so.
    pub(crate) fn slots(&mut self) -> Vec<Slot> {
        let s = self.pid1().slots().unwrap_or_default();
        let mut kept = self.kept();
        let before = kept.clone();
        if !s.fell_back.is_empty() {
            let h = kept.of(&s.fell_back);
            if h.last == Attempt::Pending {
                h.last = Attempt::RolledBack;
                h.reason = "it did not confirm at its IPL".into();
                h.time = now();
            }
        }
        if !s.booted.is_empty() && s.pending.is_empty() {
            let i = usize::from(s.booted == "B");
            let confirmed = s.confirmed.get(i).cloned().unwrap_or_default();
            let h = kept.of(&s.booted);
            if h.last == Attempt::Pending && h.release == confirmed {
                h.last = Attempt::Confirmed;
                h.time = now();
            }
        }
        if kept != before {
            self.keep(&kept);
        }
        ["A", "B"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let h = kept.of(name).clone();
                let running = s.booted == *name;
                let confirmed = s.confirmed.get(i).cloned().unwrap_or_default();
                let (release, build, fixes) =
                    if running && h.release != self.host.release.to_string() {
                        (
                            self.host.release.to_string(),
                            self.host.build.to_string(),
                            Vec::new(),
                        )
                    } else if h.release.is_empty() {
                        (confirmed, String::new(), Vec::new())
                    } else {
                        (h.release.clone(), h.build.clone(), h.fixes.clone())
                    };
                Slot {
                    name: (*name).to_string(),
                    release,
                    build,
                    fixes,
                    running,
                    default: s.default == *name,
                    last: if s.pending == *name {
                        Attempt::Pending
                    } else {
                        h.last
                    },
                    reason: h.reason,
                    time: h.time,
                }
            })
            .collect()
    }

    /// The slot that does not run; none without a record.
    pub(crate) fn other(&self) -> Option<String> {
        let s = self.pid1().slots().ok()?;
        match s.booted.as_str() {
            "A" => Some("B".into()),
            "B" => Some("A".into()),
            _ => None,
        }
    }

    fn forget_staged(&mut self) {
        let _ = fs::remove_file(self.path(IMAGE));
        let _ = fs::remove_file(self.path(STAGED));
        let _ = fs::remove_file(self.path("staged.part"));
        self.staged = None;
    }
}

/// A refusal of an image as the host's message.
pub(crate) fn refusal(name: &str, r: &Refusal, running: &Running) -> Issued {
    match r {
        Refusal::Signature => msgs().signature.with(&[name]),
        Refusal::Damaged(why) => msgs().damaged.with(&[name, why]),
        Refusal::Product { theirs, .. } => msgs().product.with(&[name, theirs, &running.product]),
        Refusal::Older { theirs, ours } => {
            msgs()
                .older
                .with(&[name, &theirs.to_string(), &ours.to_string()])
        }
        Refusal::State { file, found, reads } => msgs().state.with(&[
            name,
            "its release",
            &format!("{file} is version {found}, and it reads {reads} at most"),
        ]),
    }
}

/// Whether a stage's name and size are ones an image is staged by.
fn valid_stage(name: &str, bytes: u64) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| !c.is_control())
        && bytes > 0
        && bytes <= crate::image::MAX_IMAGE
}

/// Verify the image just written to `path`, staged as `name`: kept,
/// said, and its status.
fn verify_staged(core: &Shared, name: &str, bytes: u64, allow_older: bool) -> Staged {
    let path = lock(core).path("staged.part");
    let running = lock(core).running();
    let data = fs::read(&path);
    let result = data
        .map_err(|e| vec![msgs().not_kept.with(&[name, &e.to_string()])])
        .and_then(|data| {
            verify::verify(&data, &running, allow_older)
                .map_err(|r| vec![refusal(name, &r, &running)])
        });
    let mut c = lock(core);
    let time = now();
    match result {
        Ok(v) => {
            let image = ImageInfo {
                product: v.manifest.product.clone(),
                release: v.manifest.release.clone(),
                build: v.manifest.build.clone(),
                fixes: v.manifest.fixes.clone(),
                bytes,
            };
            let kept = StagedKept {
                name: name.to_string(),
                bytes,
                image: image.clone(),
                allow_older,
                time,
            };
            let renamed = fs::rename(&path, c.path(IMAGE)).and_then(|()| {
                let b = serde_json::to_vec(&kept).map_err(io::Error::other)?;
                state::store::atomic_write(&c.path(STAGED), &b)
            });
            if let Err(e) = renamed {
                c.forget_staged();
                let error = vec![msgs().not_kept.with(&[name, &e.to_string()])];
                return refused(&mut c, name, bytes, error, time);
            }
            let staged = Staged {
                name: name.to_string(),
                bytes,
                verification: Verification::Verified,
                image: Some(image.clone()),
                error: Vec::new(),
                time,
            };
            c.staged = Some(staged.clone());
            c.allow_older = allow_older;
            c.post(
                crate::Code::Verified,
                &format!("{name}: {} build {}", image.release, image.build),
                &[],
            );
            c.publish(&Event::Verified {
                name: name.to_string(),
                image: Some(image),
                error: Vec::new(),
                time,
            });
            staged
        }
        Err(error) => {
            c.forget_staged();
            refused(&mut c, name, bytes, error, time)
        }
    }
}

fn refused(c: &mut Core, name: &str, bytes: u64, error: Vec<Issued>, time: u64) -> Staged {
    let why = error.first().map(|i| i.text.clone()).unwrap_or_default();
    c.post(crate::Code::Refused, &why, &[]);
    c.publish(&Event::Verified {
        name: name.to_string(),
        image: None,
        error: error.clone(),
        time,
    });
    Staged {
        name: name.to_string(),
        bytes,
        verification: Verification::Refused,
        image: None,
        error,
        time,
    }
}

/// Begin a stage: refused while one is staged or the service is busy.
fn begin_stage(core: &Shared, name: &str, bytes: u64) -> Result<(), Vec<Issued>> {
    let mut c = lock(core);
    if !valid_stage(name, bytes) {
        return Err(vec![
            msgs().name.with(&[&format!("{name:?}, {bytes} bytes")]),
        ]);
    }
    if let Some(s) = &c.staged {
        return Err(vec![msgs().staged_already.with(&[&s.name])]);
    }
    if let Some(b) = &c.busy {
        return Err(vec![msgs().busy.with(&[b])]);
    }
    fs::create_dir_all(&c.host.dir)
        .map_err(|e| vec![msgs().not_kept.with(&[name, &e.to_string()])])?;
    c.staged = Some(Staged {
        name: name.to_string(),
        bytes,
        verification: Verification::Receiving,
        image: None,
        error: Vec::new(),
        time: now(),
    });
    Ok(())
}

/// The image's bytes from `from`, exactly `bytes`, then verified.
fn receive(
    core: &Shared,
    from: &mut dyn Read,
    name: &str,
    bytes: u64,
    allow_older: bool,
) -> Result<Staged, Vec<Issued>> {
    let path = lock(core).path("staged.part");
    let written = (|| -> io::Result<u64> {
        let mut out = fs::File::create(&path)?;
        let got = io::copy(&mut from.take(bytes), &mut out)?;
        out.sync_all()?;
        Ok(got)
    })();
    let got = match written {
        Ok(got) => got,
        Err(e) => {
            let mut c = lock(core);
            c.forget_staged();
            let error = vec![msgs().not_kept.with(&[name, &e.to_string()])];
            c.publish(&Event::Staged {
                name: name.to_string(),
                bytes: 0,
                error: error.clone(),
                time: now(),
            });
            return Err(error);
        }
    };
    let mut c = lock(core);
    if got < bytes {
        c.forget_staged();
        let error = vec![
            msgs()
                .cut
                .with(&[name, &got.to_string(), &bytes.to_string()]),
        ];
        c.publish(&Event::Staged {
            name: name.to_string(),
            bytes: got,
            error: error.clone(),
            time: now(),
        });
        return Err(error);
    }
    if let Some(s) = c.staged.as_mut() {
        s.verification = Verification::Verifying;
    }
    c.post(crate::Code::Staged, &format!("{name}, {bytes} bytes"), &[]);
    c.publish(&Event::Staged {
        name: name.to_string(),
        bytes,
        error: Vec::new(),
        time: now(),
    });
    drop(c);
    Ok(verify_staged(core, name, bytes, allow_older))
}

/// One connection's handler.
struct Session<'a>(&'a Shared);

impl Updates for Session<'_> {
    fn status(&mut self) -> Status {
        let mut c = lock(self.0);
        Status {
            slots: c.slots(),
            staged: c.staged.clone(),
            installing: c.installing.clone(),
        }
    }

    // The connection's: see `connection`.
    fn stage(&mut self, _: String, _: u64, _: bool) -> Result<(), Vec<Issued>> {
        Err(Vec::new())
    }

    fn stage_file(&mut self, path: String, allow_older: bool) -> Result<Staged, Vec<Issued>> {
        let p = Path::new(&path);
        let name = p
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        let mut file =
            fs::File::open(p).map_err(|e| vec![msgs().not_kept.with(&[&name, &e.to_string()])])?;
        let bytes = file.metadata().map_or(0, |m| m.len());
        begin_stage(self.0, &name, bytes)?;
        receive(self.0, &mut file, &name, bytes, allow_older)
    }

    fn discard(&mut self) -> Result<(), Vec<Issued>> {
        let mut c = lock(self.0);
        if let Some(b) = &c.busy {
            return Err(vec![msgs().busy.with(&[b])]);
        }
        let Some(s) = c.staged.clone() else {
            return Err(vec![msgs().nothing_staged.issue()]);
        };
        c.forget_staged();
        c.post(crate::Code::Discarded, &s.name, &[]);
        Ok(())
    }

    fn install(&mut self) -> Result<String, Vec<Issued>> {
        steps::install(self.0)
    }

    fn activate(&mut self, shut_down: bool, timeout: u32, by: String) -> Result<(), Vec<Issued>> {
        steps::activate(self.0, shut_down, timeout, &by)
    }

    fn cancel(&mut self) -> Result<(), Vec<Issued>> {
        steps::cancel(self.0)
    }

    fn subscribe(&mut self) {}
}

fn answer(stream: &UnixStream, bytes: &[u8]) -> io::Result<()> {
    let mut s = stream;
    ipc::frame::write_frame(&mut s, bytes).map_err(io::Error::other)
}

/// One connection: a request and its answer, a stage and its bytes, or
/// a subscription.
fn connection(core: &Shared, stream: &UnixStream) {
    let mut s = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(request) = ipc::frame::read_frame_max(&mut s, proto::MAX_FRAME) else {
        return;
    };
    let Ok(request) = ipc::decode::<Request>(&request) else {
        return;
    };
    let _ = match request {
        Request::Stage {
            name,
            bytes,
            allow_older,
        } => {
            let begun = begin_stage(core, &name, bytes);
            let ok = begun.is_ok();
            let sent = answer(stream, &ipc::encode(&begun));
            if ok && sent.is_ok() {
                // A console's stream may pause: a minute each read.
                let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
                let ended = receive(core, &mut s, &name, bytes, allow_older);
                answer(stream, &ipc::encode(&ended))
            } else {
                if ok {
                    lock(core).forget_staged();
                }
                sent
            }
        }
        Request::Subscribe {} => {
            let (tx, rx) = mpsc::channel::<Event>();
            lock(core).subscribers.push(tx);
            let mut ok = answer(stream, &ipc::encode(&()));
            while ok.is_ok() {
                match rx.recv() {
                    Ok(e) => ok = answer(stream, &ipc::encode(&e)),
                    Err(_) => break,
                }
            }
            Ok(())
        }
        other => answer(stream, &proto::dispatch(&mut Session(core), other)),
    };
    let _ = s.flush();
}

/// Whether `SIGTERM` came within `wait`.
fn term(signals: &mut Signals, wait: Duration) -> bool {
    let timeout = Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap_or(1),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    let mut fds = [PollFd::from_borrowed_fd(signals.fd(), PollFlags::IN)];
    let readable = poll(&mut fds, Some(&timeout)).is_ok() && !fds[0].revents().is_empty();
    readable && signals.read().contains(&TERM)
}

/// Run the update service for `host`. It returns only when it could
/// not start.
pub fn run(host: Host) -> String {
    let mut signals = match Signals::take(&[TERM]) {
        Ok(s) => s,
        Err(e) => return format!("the signal descriptor: {e}"),
    };
    if let Err(e) = fs::create_dir_all(&host.dir) {
        return format!("{}: {e}", host.dir.display());
    }
    // A stage cut by a restart is no stage; a verified one is kept.
    let _ = fs::remove_file(host.dir.join("staged.part"));
    let staged = fs::read(host.dir.join(STAGED))
        .ok()
        .and_then(|b| serde_json::from_slice::<StagedKept>(&b).ok())
        .filter(|_| host.dir.join(IMAGE).is_file());
    let allow_older = staged.as_ref().is_some_and(|s| s.allow_older);
    let staged = staged.map(|s| Staged {
        name: s.name,
        bytes: s.bytes,
        verification: Verification::Verified,
        image: Some(s.image),
        error: Vec::new(),
        time: s.time,
    });
    MESSAGES.get_or_init(|| host.messages);
    let socket = proto::socket(host.id, host.run_dir);
    let _ = fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => return format!("{}: {e}", socket.display()),
    };
    if let Err(e) = (host.share)(&socket) {
        return format!("{}: {e}", socket.display());
    }
    let core: Shared = Arc::new(Mutex::new(Core {
        host,
        staged,
        allow_older,
        busy: None,
        installing: None,
        subscribers: Vec::new(),
    }));
    let for_socket = core.clone();
    let _ = std::thread::Builder::new()
        .name("socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let core = for_socket.clone();
                let _ = std::thread::Builder::new()
                    .name("connection".into())
                    .spawn(move || connection(&core, &stream));
            }
        });
    if let Err(e) = supervise::ready::signal() {
        return format!("the readiness pipe: {e}");
    }
    loop {
        // What PID 1 settled at this IPL is kept.
        let _ = lock(&core).slots();
        if term(&mut signals, Duration::from_secs(5)) {
            std::process::exit(0);
        }
    }
}
