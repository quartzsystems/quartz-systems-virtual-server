// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The running host: one loop over the signals, the control socket,
//! the power button and the services' pipes. Nothing here blocks for
//! longer than a client of the control socket is given.

mod control;
mod shutdown;
pub mod table;

use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::net::UnixListener;
use std::time::{Duration, Instant};

use appliance::time::{Zone, now_secs};
use logd::client::Sender;
use logd::{Record, Severity};
use refcode::RefCode;
use refcode::post::{Posting, SOURCE, posting};
use release::fact;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use supervise::run::{Occurrence, Supervisor, Token};
use supervise::signal::{CHLD, INT, Signals, TERM};

use crate::console::{Console, screen};
use crate::slot::Slots;
use crate::sys::{button, reap};
pub use shutdown::Kind;

/// Seconds between heartbeats on the serial port.
const HEARTBEAT: Duration = Duration::from_secs(5);
/// The longest the loop sleeps: the log queue is flushed and the
/// display's clock looked at this often.
const TICK: Duration = Duration::from_secs(1);
/// What each service is given to end before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// What a descriptor that became readable is.
#[derive(Clone, Copy)]
enum Ready {
    Signal,
    Control,
    Button(usize),
    Service(Token),
}

/// The facts the status screen and the control socket show, besides
/// the ones read when asked.
pub struct Facts {
    /// The slot this IPL came from: `A`, `B`, `media` or `unknown`.
    pub slot: String,
    /// The host name.
    pub host: String,
    /// The system serial number.
    pub serial: String,
    /// The display zone.
    pub zone: Zone,
    /// The management line, as the network service last noted it.
    pub management: String,
    /// What is mounted at `/data`: the device, or `memory`.
    pub data: String,
    /// What the KVM probe found.
    pub kvm: String,
}

/// The host.
pub struct Host {
    console: Console,
    log: Sender,
    signals: Signals,
    /// The current reference code and its text.
    reference: String,
    /// Set once the IPL reaches the services.
    sup: Option<Supervisor>,
    listener: Option<UnixListener>,
    buttons: Vec<File>,
    /// The record and the slot rule; `None` on installation media.
    pub slots: Option<Slots>,
    /// What the screen shows.
    pub facts: Facts,
    /// Whether `/data` is the data partition.
    pub data_mounted: bool,
    ready: bool,
    shutdown: Option<Kind>,
    heartbeat_at: Instant,
    /// The screen as last drawn on the serial port, without its date.
    drawn: Vec<String>,
    /// The date last drawn on the display.
    drawn_date: String,
}

impl Host {
    /// A host that can post: the console is open, the signals are
    /// taken, and records wait for the log service.
    pub fn new(console: Console, signals: Signals, log_socket: &std::path::Path) -> Self {
        Self {
            console,
            log: Sender::new(log_socket),
            signals,
            reference: String::new(),
            sup: None,
            listener: None,
            buttons: Vec::new(),
            slots: None,
            facts: Facts {
                slot: "unknown".into(),
                host: String::new(),
                serial: String::new(),
                zone: Zone::UTC,
                management: "none: the network service has not started".into(),
                data: "memory".into(),
                kvm: String::new(),
            },
            data_mounted: false,
            ready: false,
            shutdown: None,
            heartbeat_at: Instant::now() + HEARTBEAT,
            drawn: Vec::new(),
            drawn_date: String::new(),
        }
    }

    fn take(&mut self, posted: Posting) {
        self.console.line(&posted.line);
        self.reference = posted
            .line
            .strip_prefix(refcode::WORD)
            .unwrap_or(&posted.line)
            .trim_start()
            .to_string();
        self.log.send(posted.record);
    }

    /// Post a reference code: the console line and the history record.
    pub fn post(&mut self, code: &RefCode, detail: &str) {
        self.take(posting(code, detail));
    }

    /// One line on the console that is no reference code, logged too.
    pub fn line(&mut self, line: &str) {
        self.console.line(line);
        self.say(Severity::Info, line);
    }

    /// A record of PID 1's own in the log.
    pub fn say(&mut self, severity: Severity, message: &str) {
        self.log.send(Record::new(SOURCE, severity, message));
    }

    /// Start the service table and open the control socket: from here
    /// the loop runs.
    pub fn start(&mut self, mut sup: Supervisor, listener: Option<UnixListener>) {
        let started = sup.advance(Instant::now());
        self.sup = Some(sup);
        self.listener = listener;
        self.buttons = button::open();
        self.occurred(started);
    }

    fn occurred(&mut self, occurrences: Vec<Occurrence>) {
        for occurrence in occurrences {
            match occurrence {
                Occurrence::Started { service, pid } => {
                    self.say(Severity::Info, &format!("{service} started, process {pid}"));
                }
                Occurrence::Ready { service } => {
                    self.say(Severity::Info, &format!("{service} is ready"));
                }
                Occurrence::AllReady => self.base_ready(),
                Occurrence::Failed {
                    service,
                    how,
                    restart_in,
                } => {
                    let next = restart_in.map_or_else(
                        || "not started again".to_string(),
                        |d| format!("started again in {} s", d.as_secs()),
                    );
                    self.say(Severity::Warning, &format!("{service}: {how}; {next}"));
                }
                Occurrence::GaveUp { service } => self.post(&refcode::SERVICE_STOPPED, service),
                Occurrence::Output {
                    service,
                    error,
                    line,
                } => {
                    let severity = if error {
                        Severity::Warning
                    } else {
                        Severity::Info
                    };
                    self.log.send(Record::new(service, severity, &line));
                }
                // The loop sees the end itself.
                Occurrence::Ended => {}
            }
        }
    }

    /// Every service is ready: the base is. A slot on trial confirms.
    fn base_ready(&mut self) {
        self.ready = true;
        let up = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|t| t.split_ascii_whitespace().next().map(str::to_string))
            .unwrap_or_else(|| "?".into());
        self.post(
            &refcode::BASE_READY,
            &format!("{up} s after the kernel started"),
        );
        #[cfg(test_hooks)]
        if crate::hooks::has(crate::hooks::REFUSE_CONFIRM) {
            self.line("TEST HOOK: the slot is not confirmed");
            return;
        }
        if let Some(mut slots) = self.slots.take() {
            let mut posted = Vec::new();
            slots.base_ready(|code, detail| posted.push(posting(code, detail)));
            self.slots = Some(slots);
            for p in posted {
                self.take(p);
            }
        }
        // A button that appeared after the IPL looked.
        if self.buttons.is_empty() {
            self.buttons = button::open();
        }
    }

    /// The status: the facts in the screen's order, then the services.
    pub fn status_now(&self) -> supervise::control::HostStatus {
        let now = now_secs();
        let record = self
            .slots
            .as_ref()
            .map_or_else(|| "no A/B record".to_string(), Slots::summary);
        let image = if cfg!(test_hooks) { "test" } else { "release" };
        let facts = [
            (fact::PRODUCT, release::NAME.to_string()),
            (fact::RELEASE, release::RELEASE.to_string()),
            (fact::BUILD, release::BUILD_ID.to_string()),
            (fact::IMAGE, image.to_string()),
            (fact::SLOT, self.facts.slot.clone()),
            (fact::RECORD, record),
            (fact::HOST, self.facts.host.clone()),
            (fact::SERIAL, self.facts.serial.clone()),
            (fact::MANAGEMENT, self.facts.management.clone()),
            (fact::TIME, now.to_string()),
            (fact::DATE, self.facts.zone.display(now)),
            (fact::REFERENCE, self.reference.clone()),
            (fact::DATA, self.facts.data.clone()),
            (
                fact::READY,
                if self.ready { "yes" } else { "no" }.to_string(),
            ),
            (fact::KVM, self.facts.kvm.clone()),
        ];
        supervise::control::HostStatus {
            facts: facts.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            services: self
                .sup
                .as_ref()
                .map(Supervisor::status)
                .unwrap_or_default(),
        }
    }

    /// Draw the screen where it changed: on the serial port when
    /// anything but the date did, on the display for the date too.
    fn draw(&mut self) {
        let status = self.status_now();
        let date = status.fact(fact::DATE).unwrap_or("").to_string();
        let lines = screen::lines(&release::banner(), &status);
        let undated: Vec<String> = lines
            .iter()
            .filter(|l| !l.ends_with(&date) || date.is_empty())
            .cloned()
            .collect();
        let changed = undated != self.drawn;
        if changed {
            self.console.serial_screen(&lines);
            self.drawn = undated;
        }
        if changed || date != self.drawn_date {
            self.console.display_screen(&lines);
            self.drawn_date = date;
        }
    }

    /// Wait for something to do, at most until `deadline`.
    fn wait(&mut self, deadline: Instant) -> Vec<Ready> {
        let left = deadline.saturating_duration_since(Instant::now());
        let timeout = Timespec {
            tv_sec: i64::try_from(left.as_secs()).unwrap_or(1),
            tv_nsec: i64::from(left.subsec_nanos()),
        };
        let mut what = vec![Ready::Signal];
        let mut fds = vec![PollFd::from_borrowed_fd(self.signals.fd(), PollFlags::IN)];
        if let Some(listener) = &self.listener {
            what.push(Ready::Control);
            fds.push(PollFd::from_borrowed_fd(listener.as_fd(), PollFlags::IN));
        }
        for (i, device) in self.buttons.iter().enumerate() {
            what.push(Ready::Button(i));
            fds.push(PollFd::from_borrowed_fd(device.as_fd(), PollFlags::IN));
        }
        if let Some(sup) = &self.sup {
            for (token, fd) in sup.fds() {
                what.push(Ready::Service(token));
                fds.push(PollFd::from_borrowed_fd(fd, PollFlags::IN));
            }
        }
        if poll(&mut fds, Some(&timeout)).is_err() {
            return Vec::new();
        }
        what.into_iter()
            .zip(&fds)
            .filter(|(_, fd)| !fd.revents().is_empty())
            .map(|(ready, _)| ready)
            .collect()
    }

    fn signalled(&mut self, now: Instant) {
        for signal in self.signals.read() {
            match signal {
                CHLD => {
                    for (pid, how) in reap::ended() {
                        if let Some(sup) = &mut self.sup {
                            let (_, occurrences) = sup.exited(pid, &how, now);
                            self.occurred(occurrences);
                        }
                    }
                }
                TERM => self.begin_shutdown(Kind::PowerOff, "SIGTERM"),
                // Ctrl-Alt-Del, which the kernel hands to PID 1.
                INT => self.begin_shutdown(Kind::Restart, "Ctrl-Alt-Del"),
                _ => {}
            }
        }
    }

    /// One turn of the loop.
    fn turn(&mut self) {
        let mut deadline = Instant::now() + TICK;
        deadline = deadline.min(self.heartbeat_at);
        if let Some(at) = self.sup.as_ref().and_then(Supervisor::deadline) {
            deadline = deadline.min(at);
        }
        let ready = self.wait(deadline);
        let now = Instant::now();
        for what in ready {
            match what {
                Ready::Signal => self.signalled(now),
                Ready::Control => self.serve_control(),
                Ready::Button(i) => {
                    if self.buttons.get(i).is_some_and(button::pressed) {
                        self.begin_shutdown(Kind::PowerOff, "the power button");
                    }
                }
                Ready::Service(token) => {
                    if let Some(sup) = &mut self.sup {
                        let occurrences = sup.readable(token, now);
                        self.occurred(occurrences);
                    }
                }
            }
        }
        if let Some(sup) = &mut self.sup {
            let occurrences = sup.advance(now);
            self.occurred(occurrences);
        }
        if now >= self.heartbeat_at {
            self.console.serial(&refcode::HEARTBEAT.line());
            self.heartbeat_at += HEARTBEAT;
        }
        self.log.flush();
        if self.shutdown.is_some() {
            if self.sup.as_ref().is_none_or(Supervisor::ended) {
                self.finish_shutdown();
            }
        } else {
            self.draw();
        }
    }

    /// Run for ever: PID 1 must not exit.
    pub fn run(mut self) -> ! {
        loop {
            self.turn();
        }
    }
}
