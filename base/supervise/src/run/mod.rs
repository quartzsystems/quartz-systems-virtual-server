// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The supervisor. It owns no loop: PID 1 polls the descriptors it
//! names, tells it what was readable and which child ended, and calls
//! [`Supervisor::advance`] when its deadline passes. Everything that
//! happens comes back as [`Occurrence`]s for PID 1 to log and post.

mod pipes;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};

use crate::control::ServiceStatus;
use crate::ready::READY;
use crate::table::{self, Policy, Service, Verdict};
use pipes::Stream;

/// Which descriptor of which service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    /// The readiness pipe of the service at this table index.
    Ready(usize),
    /// Its standard output.
    Out(usize),
    /// Its standard error.
    Err(usize),
}

/// Something that happened to a service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Occurrence {
    /// It was started.
    Started {
        /// The service.
        service: &'static str,
        /// Its process.
        pid: u32,
    },
    /// It said it is ready.
    Ready {
        /// The service.
        service: &'static str,
    },
    /// Every service of the table is ready, for the first time.
    AllReady,
    /// It ended, or could not be started.
    Failed {
        /// The service.
        service: &'static str,
        /// How it ended.
        how: String,
        /// When it is started again; `None` when it is not.
        restart_in: Option<Duration>,
    },
    /// It failed too often and stays stopped.
    GaveUp {
        /// The service.
        service: &'static str,
    },
    /// A line it wrote.
    Output {
        /// The service.
        service: &'static str,
        /// Whether it was on standard error.
        error: bool,
        /// The line, without its end.
        line: String,
    },
    /// It did not end when asked to during the orderly stop, and was
    /// killed.
    Killed {
        /// The service.
        service: &'static str,
    },
    /// The orderly stop is complete: no service is left.
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Not started yet: something it needs is not ready.
    Waiting,
    /// Running, not ready yet.
    Starting,
    /// Running and ready.
    Ready,
    /// Failed; started again at `at`.
    Restarting { at: Instant },
    /// Failed too often.
    Stopped,
    /// Asked to end; killed at `kill_at`.
    Stopping { kill_at: Instant },
    /// Ended by the orderly stop.
    Ended,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Waiting => "waiting",
            State::Starting => "starting",
            State::Ready => "ready",
            State::Restarting { .. } => "restarting",
            State::Stopped => "stopped",
            State::Stopping { .. } => "stopping",
            State::Ended => "ended",
        }
    }

    fn running(self) -> bool {
        matches!(
            self,
            State::Starting | State::Ready | State::Stopping { .. }
        )
    }
}

struct Entry {
    state: State,
    policy: Policy,
    pid: u32,
    started: Instant,
    /// Killed for not becoming ready in time.
    late: bool,
    ready: Option<File>,
    out: Stream,
    err: Stream,
}

/// The services of one table.
pub struct Supervisor {
    table: &'static [Service],
    order: Vec<usize>,
    entries: Vec<Entry>,
    /// The grace each service gets once the orderly stop began.
    stopping: Option<Duration>,
    /// Services ordered after this are not started yet.
    held: Option<u32>,
    /// The orderly stop leaves services ordered up to this running.
    floor: u32,
    announced_ready: bool,
    announced_end: bool,
}

fn signal(pid: u32, signal: Signal) {
    if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
        let _ = kill_process(pid, signal);
    }
}

impl Supervisor {
    /// A supervisor for `table`; nothing is started yet.
    pub fn new(table: &'static [Service], now: Instant) -> io::Result<Self> {
        table::check(table).map_err(io::Error::other)?;
        let entries = table
            .iter()
            .map(|_| {
                Ok(Entry {
                    state: State::Waiting,
                    policy: Policy::default(),
                    pid: 0,
                    started: now,
                    late: false,
                    ready: None,
                    out: Stream::new()?,
                    err: Stream::new()?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            table,
            order: table::start_order(table),
            entries,
            stopping: None,
            held: None,
            floor: 0,
            announced_ready: false,
            announced_end: false,
        })
    }

    fn is_ready(&self, name: &str) -> bool {
        self.table
            .iter()
            .zip(&self.entries)
            .any(|(s, e)| s.name == name && e.state == State::Ready)
    }

    fn spawn(&mut self, i: usize, now: Instant) -> Occurrence {
        let service = &self.table[i];
        let spawned = (|| {
            let (ready_read, ready_write) = pipes::pipe()?;
            let entry = &self.entries[i];
            let mut command = Command::new(service.binary);
            command
                .args(service.args)
                .env_clear()
                .stdin(Stdio::from(ready_write))
                .stdout(Stdio::from(entry.out.writer()?))
                .stderr(Stdio::from(entry.err.writer()?));
            // PID 1 blocks the signals it takes from a descriptor; a
            // service must not inherit that, or SIGTERM cannot end it.
            let child = crate::signal::unblocked(&mut command).spawn()?;
            // PID 1 reaps every child itself; the handle is not waited on.
            Ok::<_, io::Error>((child.id(), ready_read))
        })();
        let entry = &mut self.entries[i];
        entry.started = now;
        entry.late = false;
        match spawned {
            Ok((pid, ready)) => {
                entry.pid = pid;
                entry.ready = Some(ready);
                entry.state = State::Starting;
                Occurrence::Started {
                    service: service.name,
                    pid,
                }
            }
            Err(e) => self.failure(i, format!("could not be started: {e}"), now),
        }
    }

    fn failure(&mut self, i: usize, how: String, now: Instant) -> Occurrence {
        let entry = &mut self.entries[i];
        entry.pid = 0;
        entry.ready = None;
        let restart_in = match entry.policy.failed(entry.started, now) {
            Verdict::Restart(delay) => {
                entry.state = State::Restarting { at: now + delay };
                Some(delay)
            }
            Verdict::Stop => {
                entry.state = State::Stopped;
                None
            }
        };
        Occurrence::Failed {
            service: self.table[i].name,
            how,
            restart_in,
        }
    }

    fn after_failure(&self, i: usize, failed: Occurrence, out: &mut Vec<Occurrence>) {
        out.push(failed);
        if self.entries[i].state == State::Stopped {
            out.push(Occurrence::GaveUp {
                service: self.table[i].name,
            });
        }
    }

    /// Do what is due at `now`: start what can start, kill what did not
    /// become ready, and carry the orderly stop forward.
    pub fn advance(&mut self, now: Instant) -> Vec<Occurrence> {
        let mut out = Vec::new();
        if let Some(grace) = self.stopping {
            self.advance_stop(now, grace, &mut out);
            return out;
        }
        for i in self.order.clone() {
            let service = &self.table[i];
            if self.held.is_some_and(|held| service.order > held) {
                continue;
            }
            match self.entries[i].state {
                State::Waiting => {}
                State::Restarting { at } if at <= now => {}
                State::Starting if now >= self.entries[i].started + service.ready_within => {
                    if !self.entries[i].late {
                        self.entries[i].late = true;
                        signal(self.entries[i].pid, Signal::KILL);
                    }
                    continue;
                }
                _ => continue,
            }
            if service.needs.iter().all(|n| self.is_ready(n)) {
                let started = self.spawn(i, now);
                if matches!(started, Occurrence::Failed { .. }) {
                    self.after_failure(i, started, &mut out);
                } else {
                    out.push(started);
                }
            }
        }
        out
    }

    fn advance_stop(&mut self, now: Instant, grace: Duration, out: &mut Vec<Occurrence>) {
        let floor = self.floor;
        for (s, e) in self.table.iter().zip(&mut self.entries) {
            if !e.state.running() && s.order > floor {
                e.state = State::Ended;
            }
        }
        // One at a time, the last started first.
        let last = self
            .order
            .iter()
            .rev()
            .copied()
            .find(|i| self.entries[*i].state.running() && self.table[*i].order > floor);
        match last {
            Some(i) => {
                let e = &mut self.entries[i];
                match e.state {
                    State::Stopping { kill_at } if now >= kill_at => {
                        signal(e.pid, Signal::KILL);
                        e.state = State::Stopping {
                            kill_at: now + grace,
                        };
                        out.push(Occurrence::Killed {
                            service: self.table[i].name,
                        });
                    }
                    State::Stopping { .. } => {}
                    _ => {
                        signal(e.pid, Signal::TERM);
                        e.state = State::Stopping {
                            kill_at: now + grace,
                        };
                    }
                }
            }
            None if !self.announced_end && floor == 0 => {
                self.announced_end = true;
                out.push(Occurrence::Ended);
            }
            None => {}
        }
    }

    /// When [`Supervisor::advance`] next has something to do.
    pub fn deadline(&self) -> Option<Instant> {
        self.table
            .iter()
            .zip(&self.entries)
            .filter_map(|(s, e)| match e.state {
                State::Restarting { at } => Some(at),
                State::Starting if !e.late => Some(e.started + s.ready_within),
                State::Stopping { kill_at } => Some(kill_at),
                _ => None,
            })
            .min()
    }

    /// The descriptors to poll for reading.
    pub fn fds(&self) -> Vec<(Token, BorrowedFd<'_>)> {
        let mut fds = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(ready) = &e.ready {
                fds.push((Token::Ready(i), ready.as_fd()));
            }
            fds.push((Token::Out(i), e.out.fd()));
            fds.push((Token::Err(i), e.err.fd()));
        }
        fds
    }

    /// The descriptor of `token` is readable.
    pub fn readable(&mut self, token: Token, now: Instant) -> Vec<Occurrence> {
        let mut out = Vec::new();
        match token {
            Token::Ready(i) => self.read_ready(i, now, &mut out),
            Token::Out(i) | Token::Err(i) => {
                let error = matches!(token, Token::Err(_));
                let Some(entry) = self.entries.get_mut(i) else {
                    return out;
                };
                let stream = if error {
                    &mut entry.err
                } else {
                    &mut entry.out
                };
                let service = self.table[i].name;
                out.extend(stream.lines().into_iter().map(|line| Occurrence::Output {
                    service,
                    error,
                    line,
                }));
            }
        }
        out
    }

    fn read_ready(&mut self, i: usize, now: Instant, out: &mut Vec<Occurrence>) {
        let Some(entry) = self.entries.get_mut(i) else {
            return;
        };
        let Some(pipe) = &entry.ready else { return };
        match pipes::read_byte(pipe) {
            // Nothing yet.
            Ok(None) => return,
            Ok(Some(READY)) if entry.state == State::Starting => {
                entry.state = State::Ready;
                out.push(Occurrence::Ready {
                    service: self.table[i].name,
                });
            }
            // Closed without the byte, or something else: the exit says the rest.
            _ => {}
        }
        entry.ready = None;
        if !self.announced_ready && self.all_ready() {
            self.announced_ready = true;
            out.push(Occurrence::AllReady);
        }
        // What waited for this one starts now.
        out.extend(self.advance(now));
    }

    /// The child `pid` ended (`how` says how). False when it is not a
    /// service: PID 1 reaps orphans too.
    pub fn exited(&mut self, pid: u32, how: &str, now: Instant) -> (bool, Vec<Occurrence>) {
        let mut out = Vec::new();
        let Some(i) = self
            .entries
            .iter()
            .position(|e| e.pid == pid && e.state.running())
        else {
            return (false, out);
        };
        if self.stopping.is_some() && self.table[i].order > self.floor {
            self.entries[i].state = State::Ended;
            self.entries[i].pid = 0;
            out.extend(self.advance(now));
            return (true, out);
        }
        let how = if self.entries[i].late {
            format!("was not ready in time ({how})")
        } else {
            how.to_string()
        };
        let failed = self.failure(i, how, now);
        self.after_failure(i, failed, &mut out);
        (true, out)
    }

    /// Begin the orderly stop: services end in reverse start order,
    /// each given `grace` before it is killed.
    pub fn begin_stop(&mut self, grace: Duration, now: Instant) -> Vec<Occurrence> {
        self.begin_stop_above(0, grace, now)
    }

    /// The orderly stop of the services ordered after `floor` only:
    /// the caller has something to do (unmount a file system) before
    /// the ones up to `floor` may end. [`Supervisor::begin_stop`] then
    /// ends those.
    pub fn begin_stop_above(
        &mut self,
        floor: u32,
        grace: Duration,
        now: Instant,
    ) -> Vec<Occurrence> {
        self.stopping = Some(grace);
        self.floor = floor;
        self.advance(now)
    }

    /// Whether no service ordered after `floor` runs any more.
    pub fn stopped_above(&self, floor: u32) -> bool {
        self.table
            .iter()
            .zip(&self.entries)
            .all(|(s, e)| s.order <= floor || !e.state.running())
    }

    /// Start only the services ordered up to `order` for now: the
    /// caller has something to do (mount a file system) once they are
    /// ready and before the others start.
    pub fn hold_after(&mut self, order: u32) {
        self.held = Some(order);
    }

    /// Start the services that were held back.
    pub fn release(&mut self, now: Instant) -> Vec<Occurrence> {
        self.held = None;
        self.advance(now)
    }

    /// Whether the services ordered up to `order` have all settled:
    /// each is ready, or failed too often to be started again.
    pub fn settled_up_to(&self, order: u32) -> bool {
        self.table
            .iter()
            .zip(&self.entries)
            .filter(|(s, _)| s.order <= order)
            .all(|(_, e)| matches!(e.state, State::Ready | State::Stopped))
    }

    /// The state of the service named `name`, as status words it.
    pub fn state_of(&self, name: &str) -> Option<&'static str> {
        self.table
            .iter()
            .zip(&self.entries)
            .find(|(s, _)| s.name == name)
            .map(|(_, e)| e.state.name())
    }

    /// Whether every service is ready.
    pub fn all_ready(&self) -> bool {
        self.entries.iter().all(|e| e.state == State::Ready)
    }

    /// Whether the orderly stop is complete.
    pub fn ended(&self) -> bool {
        self.announced_end
    }

    /// Every service, in start order.
    pub fn status(&self) -> Vec<ServiceStatus> {
        self.order
            .iter()
            .map(|i| {
                let e = &self.entries[*i];
                ServiceStatus {
                    name: self.table[*i].name.to_string(),
                    state: e.state.name().to_string(),
                    pid: if e.state.running() { e.pid } else { 0 },
                    failures: e.policy.total(),
                }
            })
            .collect()
    }
}
