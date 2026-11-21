// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The problem service on Linux. Every second it reads the reference
//! code history on from where it had got to, takes each code into the
//! log, keeps the log with its new place in one write, and only then
//! tells its subscribers: a service started again loses no code and
//! takes none twice. Every minute it looks at how full `/data` is.

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use supervise::control::{About, about as kind};
use supervise::signal::{Signals, TERM};

use crate::book::Book;
use crate::proto::{self, Problems, Request};
use crate::record::{Filter, Line, Noticed, Page, Problem, RatedPage, RatedProblem};

/// What the service is given by the product it runs as.
#[derive(Clone)]
pub struct Host {
    /// The product's table, severity and messages.
    pub rules: crate::table::Rules,
    /// Post the product's code that the data file system is nearly full
    /// (`true`) or has room again, with the detail and what it is about.
    pub post_data: fn(bool, &str, &[About]),
    /// The code `post_data` posts when it is nearly full: a problem it
    /// opened says so at the first look.
    pub full_code: &'static str,
    /// The product's id (`appliance.id`): the socket's name.
    pub id: &'static str,
    /// The run directory: the socket.
    pub run_dir: &'static str,
    /// Its own directory: the log.
    pub dir: PathBuf,
    /// The reference code history.
    pub history: PathBuf,
    /// The data file system, whose fullness is looked at.
    pub data: PathBuf,
    /// Share the socket with the QSI server's group.
    pub share: fn(&std::path::Path) -> std::io::Result<()>,
}

/// How often the history is read.
const TURN: Duration = Duration::from_secs(1);
/// How often `/data` is looked at.
const LOOK: Duration = Duration::from_secs(60);
/// The share of `/data` used, in percent, that opens a problem.
pub const DATA_FULL: u64 = 90;
/// The share below which the problem is cleared.
pub const DATA_ROOM: u64 = 85;

/// The log and who is told of it.
struct Core {
    book: Book,
    host: Host,
    subscribers: Vec<mpsc::Sender<Noticed>>,
}

type Shared = Arc<Mutex<Core>>;

fn lock(core: &Shared) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now() -> u64 {
    u64::try_from(appliance::time::now_secs()).unwrap_or(0)
}

impl Core {
    /// No problem `number`, in the product's words.
    fn no_problem(&self, number: u64) -> Vec<msg::Issued> {
        vec![
            self.host
                .rules
                .messages
                .no_problem
                .with(&[&number.to_string()]),
        ]
    }

    fn publish(&mut self, events: &[Noticed]) {
        self.subscribers
            .retain(|tx| events.iter().all(|e| tx.send(e.clone()).is_ok()));
    }

    /// Keep the log; said when it could not be.
    fn keep(&self) -> bool {
        match self.book.keep(&self.host.dir) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("the problem log was not written: {e}");
                false
            }
        }
    }

    /// Read the history on; what happened is kept, then told.
    fn turn(&mut self) {
        let Ok((postings, at)) = crate::feed::read(&self.host.history, self.book.cursor()) else {
            return;
        };
        if at == self.book.cursor() {
            return;
        }
        let before = self.book.clone();
        let t = now();
        let mut events = Vec::new();
        for p in &postings {
            events.extend(self.book.take(&self.host.rules, p, t));
        }
        self.book.read_to(at);
        if self.keep() {
            self.publish(&events);
        } else {
            // Not kept: read again at the next turn.
            self.book = before;
        }
    }
}

/// One connection's handler.
struct Session<'a>(&'a Shared);

impl Problems for Session<'_> {
    fn list(&mut self, filter: Filter, before: Option<u64>, limit: u32) -> Page {
        lock(self.0).book.list(&filter, before, limit)
    }

    fn get(&mut self, number: u64) -> Result<Problem, Vec<msg::Issued>> {
        let core = lock(self.0);
        core.book.get(number).ok_or_else(|| core.no_problem(number))
    }

    fn close(
        &mut self,
        number: u64,
        anyway: bool,
        by: String,
    ) -> Result<Problem, Vec<msg::Issued>> {
        let mut core = lock(self.0);
        let rules = core.host.rules;
        let before = core.book.clone();
        let closed = core.book.close(&rules, number, anyway, &by, now())?;
        if !core.keep() {
            core.book = before;
            return Err(vec![rules.messages.not_kept.issue()]);
        }
        eprintln!("problem {number} closed by {}", closed.problem.closed_by);
        core.publish(std::slice::from_ref(&closed));
        core.book.get(number).ok_or_else(|| core.no_problem(number))
    }

    fn subscribe(&mut self) {}

    fn note(&mut self, number: u64, text: String, by: String) -> Result<Line, Vec<msg::Issued>> {
        let mut core = lock(self.0);
        let rules = core.host.rules;
        let before = core.book.clone();
        let noted = core.book.note(&rules, number, &text, &by, now())?;
        if !core.keep() {
            core.book = before;
            return Err(vec![rules.messages.not_kept.issue()]);
        }
        eprintln!("problem {number}: a note by {by}");
        core.publish(std::slice::from_ref(&noted));
        Ok(noted.note.map(|n| n.line).unwrap_or_default())
    }

    fn list_rated(&mut self, filter: Filter, before: Option<u64>, limit: u32) -> RatedPage {
        let core = lock(self.0);
        core.book
            .list_rated(&core.host.rules, &filter, before, limit)
    }

    fn get_rated(&mut self, number: u64) -> Result<RatedProblem, Vec<msg::Issued>> {
        let core = lock(self.0);
        core.book
            .get_rated(&core.host.rules, number)
            .ok_or_else(|| core.no_problem(number))
    }
}

fn answer(stream: &UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    let mut s = stream;
    ipc::frame::write_frame(&mut s, bytes).map_err(std::io::Error::other)
}

/// One connection: a request and its answer, or a subscription.
fn connection(core: &Shared, stream: &UnixStream) {
    let mut s = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(request) = ipc::frame::read_frame_max(&mut s, proto::MAX_FRAME) else {
        return;
    };
    let Ok(request) = ipc::decode::<Request>(&request) else {
        return;
    };
    let _ = match request {
        Request::Subscribe {} => {
            let (tx, rx) = mpsc::channel::<Noticed>();
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
}

/// The share of the file system at `path` in use, in percent.
fn used_percent(path: &std::path::Path) -> Option<u64> {
    let s = rustix::fs::statvfs(path).ok()?;
    let total = s.f_blocks;
    if total == 0 {
        return None;
    }
    let free = s.f_bavail.min(total);
    Some((total - free).saturating_mul(100) / total)
}

/// Look at `/data`: past [`DATA_FULL`] the problem's code is posted,
/// and back under [`DATA_ROOM`] its clearing code, once each way.
fn look_at_data(core: &Shared, said_full: &mut Option<bool>) {
    let host = lock(core).host.clone();
    let Some(used) = used_percent(&host.data) else {
        return;
    };
    let about = [About {
        kind: kind::SYSTEM.into(),
        name: host.data.display().to_string(),
        ..About::default()
    }];
    // What the log holds decides at the first look.
    let full = *said_full.get_or_insert_with(|| lock(core).book.holds(host.full_code));
    let detail = format!("{used}% used");
    if !full && used >= DATA_FULL {
        (host.post_data)(true, &detail, &about);
        *said_full = Some(true);
    } else if full && used < DATA_ROOM {
        (host.post_data)(false, &detail, &about);
        *said_full = Some(false);
    }
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

/// Run the problem service for `host`. It returns only when it could
/// not start.
pub fn run(host: Host) -> String {
    let mut signals = match Signals::take(&[TERM]) {
        Ok(signals) => signals,
        Err(e) => return format!("the signal descriptor: {e}"),
    };
    if let Err(e) = std::fs::create_dir_all(&host.dir) {
        return format!("{}: {e}", host.dir.display());
    }
    let book = Book::open(&host.dir);
    let socket = proto::socket(host.id, host.run_dir);
    let _ = std::fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(l) => l,
        Err(e) => return format!("{}: {e}", socket.display()),
    };
    if let Err(e) = (host.share)(&socket) {
        return format!("{}: {e}", socket.display());
    }
    let core: Shared = Arc::new(Mutex::new(Core {
        book,
        host,
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
    let mut looked: Option<Instant> = None;
    let mut said_full = None;
    loop {
        lock(&core).turn();
        if looked.is_none_or(|t| t.elapsed() >= LOOK) {
            look_at_data(&core, &mut said_full);
            looked = Some(Instant::now());
        }
        if term(&mut signals, TURN) {
            std::process::exit(0);
        }
    }
}
