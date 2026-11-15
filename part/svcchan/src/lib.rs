// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One partition's service channel on the host's side
//! (`docs/SERVICE.md`). The protocol is QS/400's crate, `qsipart`: this
//! crate keeps what the host needs of one connection and does nothing
//! itself. Bytes go in with the time; what they say comes out as
//! [`Happened`], and what is to be sent as frames.
//!
//! What the system sends is hostile. The reader is the protocol's,
//! bounded; a frame of a kind the host knows whose body does not read,
//! too many frames at once, or too much junk, and the channel is
//! [`Happened::Nonsense`]: whoever holds it closes it, and the partition
//! runs on. A request waits for the system's answer for a while, then
//! is timed out; at most a few wait at once.

use qsipart::service::{self, FromSystem, Hello, PeerHello, Ping, Refcode, State, ToSystem};
use qsipart::{Reader, caps};

/// Seconds between pings.
pub const PING_SECS: u64 = 10;
/// Pings missed before the system is not responding.
pub const MISSED: u64 = 3;
/// Seconds a request waits for its answer.
pub const ANSWER_SECS: u64 = 15;
/// The most requests that wait at once.
pub const MAX_WAITING: usize = 16;
/// The most frames taken in one second: more is a flood.
pub const MAX_FRAMES_A_SECOND: u64 = 2000;
/// The most bodies of a known kind that do not read, in a connection.
pub const MAX_BAD: u64 = 16;
/// The most bytes of junk passed over in a connection.
pub const MAX_JUNK: u64 = 1 << 20;
/// What the host's side says it can do.
pub const HOST_CAPS: u64 = caps::SERVICE_V1;

/// A request of the host's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Power down: immediate or controlled, the delay, a restart.
    PowerDown {
        /// `OPTION(*IMMED)`.
        immediate: bool,
        /// The delay, in seconds.
        delay: u32,
        /// `RESTART(*YES)`.
        restart: bool,
    },
    /// A control panel function the system carries out.
    Panel(u8),
}

/// What the system said, or what time did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Happened {
    /// Its hello: on each connection, and when what it says changed.
    Hello(Hello),
    /// A reference code it posted.
    Refcode(Refcode),
    /// Its state.
    State(State),
    /// A request was answered: accepted, or refused with the system's
    /// message identifier and text.
    Answered {
        /// The request's own number.
        tag: u64,
        /// What was asked.
        ask: Ask,
        /// Accepted, or the refusal.
        result: Result<(), (String, String)>,
    },
    /// A request had no answer in time.
    TimedOut {
        /// The request's own number.
        tag: u64,
        /// What was asked.
        ask: Ask,
    },
    /// The system asks for the host's clock: answer with [`Channel::time`].
    TimeRequest,
    /// It stopped answering pings.
    NotResponding,
    /// It answers again.
    Responding,
    /// It sent what is not the protocol: the channel is to be closed.
    Nonsense(String),
}

#[derive(Clone, Copy, Debug)]
struct Waiting {
    id: u32,
    tag: u64,
    ask: Ask,
    since: u64,
}

/// One connection's channel.
#[derive(Debug)]
pub struct Channel {
    reader: Reader,
    /// The system's last hello.
    pub hello: Option<Hello>,
    /// The version both sides speak.
    pub version: u16,
    /// The capabilities both have.
    pub caps: u64,
    next_id: u32,
    waiting: Vec<Waiting>,
    /// When a ping last went, and the last pong came.
    pinged: Option<(u32, u64)>,
    answered_at: u64,
    /// Whether it is not responding now.
    pub silent: bool,
    window: (u64, u64),
    bad: u64,
    closed: bool,
}

/// The frame of `m`; nothing when it cannot be one.
fn frame(m: &ToSystem) -> Vec<u8> {
    m.encode().unwrap_or_default()
}

impl Channel {
    /// A channel connected at `now`.
    pub fn new(now: u64) -> Self {
        Self {
            reader: Reader::new(),
            hello: None,
            version: 0,
            caps: 0,
            next_id: 1,
            waiting: Vec::new(),
            pinged: None,
            answered_at: now,
            silent: false,
            window: (now, 0),
            bad: 0,
            closed: false,
        }
    }

    /// The host's hello, `name` its name and release.
    pub fn peer_hello(name: &str) -> Vec<u8> {
        frame(&ToSystem::Hello(PeerHello {
            version: qsipart::VERSION,
            caps: HOST_CAPS,
            name: name.chars().take(64).collect(),
        }))
    }

    /// The host's clock, for a time request.
    pub fn time(unix_ns: i64) -> Vec<u8> {
        frame(&ToSystem::Time(service::Time { unix_ns }))
    }

    /// Whether the system has said hello and has `cap`.
    pub fn has(&self, cap: u64) -> bool {
        self.hello.is_some() && self.caps & cap == cap
    }

    fn nonsense(&mut self, why: String, out: &mut Vec<Happened>) {
        if !self.closed {
            self.closed = true;
            out.push(Happened::Nonsense(why));
        }
    }

    /// Bytes the system sent, at `now`.
    pub fn bytes(&mut self, data: &[u8], now: u64) -> Vec<Happened> {
        let mut out = Vec::new();
        if self.closed {
            return out;
        }
        self.reader.push(data);
        while let Some(f) = self.reader.next_frame() {
            if self.window.0 != now {
                self.window = (now, 0);
            }
            self.window.1 += 1;
            if self.window.1 > MAX_FRAMES_A_SECOND {
                self.nonsense(
                    format!("more than {MAX_FRAMES_A_SECOND} frames in a second"),
                    &mut out,
                );
                return out;
            }
            match FromSystem::decode(&f) {
                Ok(Some(m)) => self.take(m, now, &mut out),
                // A kind of a later version's: passed over.
                Ok(None) => {}
                Err(e) => {
                    self.bad += 1;
                    if self.bad > MAX_BAD {
                        self.nonsense(format!("frames that do not read: {e}"), &mut out);
                        return out;
                    }
                }
            }
        }
        if self.reader.skipped() > MAX_JUNK {
            self.nonsense(
                format!("{} bytes that are no frame", self.reader.skipped()),
                &mut out,
            );
        }
        out
    }

    fn take(&mut self, m: FromSystem, now: u64, out: &mut Vec<Happened>) {
        match m {
            FromSystem::Hello(h) => {
                self.version = qsipart::agreed(qsipart::VERSION, h.version);
                self.caps = HOST_CAPS & h.caps;
                self.answered_at = now;
                self.hello = Some(h.clone());
                out.push(Happened::Hello(h));
            }
            FromSystem::Refcode(r) => out.push(Happened::Refcode(r)),
            FromSystem::State(s) => out.push(Happened::State(s)),
            FromSystem::Ack(a) => self.answer(a.id, Ok(()), out),
            FromSystem::Refused(r) => self.answer(r.id, Err((r.msgid, r.text)), out),
            FromSystem::Pong(p) => {
                if self.pinged.is_some_and(|(id, _)| id == p.id) {
                    self.pinged = None;
                }
                self.answered_at = now;
                if self.silent {
                    self.silent = false;
                    out.push(Happened::Responding);
                }
            }
            FromSystem::TimeRequest => out.push(Happened::TimeRequest),
        }
    }

    fn answer(&mut self, id: u32, result: Result<(), (String, String)>, out: &mut Vec<Happened>) {
        if let Some(at) = self.waiting.iter().position(|w| w.id == id) {
            let w = self.waiting.remove(at);
            out.push(Happened::Answered {
                tag: w.tag,
                ask: w.ask,
                result,
            });
        }
    }

    /// Ask the system for `ask` at `now`; `tag` is the caller's, and
    /// comes back with the answer. The frame to send, or nothing when
    /// too many wait already.
    pub fn ask(&mut self, ask: Ask, tag: u64, now: u64) -> Option<Vec<u8>> {
        if self.waiting.len() >= MAX_WAITING {
            return None;
        }
        let id = self.id();
        let m = match ask {
            Ask::PowerDown {
                immediate,
                delay,
                restart,
            } => ToSystem::PowerDown(service::PowerDown {
                id,
                immediate,
                delay: delay.min(service::MAX_DELAY),
                restart,
            }),
            Ask::Panel(function) => ToSystem::Panel(service::Panel { id, function }),
        };
        self.waiting.push(Waiting {
            id,
            tag,
            ask,
            since: now,
        });
        Some(frame(&m))
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// What time does at `now`: requests timed out, a ping due, the
    /// not-responding rule. What happened, and the frames to send.
    pub fn tick(&mut self, now: u64) -> (Vec<Happened>, Vec<Vec<u8>>) {
        let mut out = Vec::new();
        let mut send = Vec::new();
        let mut i = 0;
        while i < self.waiting.len() {
            if now.saturating_sub(self.waiting[i].since) >= ANSWER_SECS {
                let w = self.waiting.remove(i);
                out.push(Happened::TimedOut {
                    tag: w.tag,
                    ask: w.ask,
                });
            } else {
                i += 1;
            }
        }
        if self.closed || self.hello.is_none() {
            return (out, send);
        }
        let due = self
            .pinged
            .is_none_or(|(_, at)| now.saturating_sub(at) >= PING_SECS);
        if due {
            let id = self.id();
            self.pinged = Some((id, now));
            send.push(frame(&ToSystem::Ping(Ping { id })));
        }
        if !self.silent && now.saturating_sub(self.answered_at) >= PING_SECS * MISSED {
            self.silent = true;
            out.push(Happened::NotResponding);
        }
        (out, send)
    }

    /// The requests that still wait, by their tags: the channel went
    /// away under them.
    pub fn abandon(&mut self) -> Vec<(u64, Ask)> {
        self.waiting.drain(..).map(|w| (w.tag, w.ask)).collect()
    }
}

#[cfg(test)]
mod tests;
