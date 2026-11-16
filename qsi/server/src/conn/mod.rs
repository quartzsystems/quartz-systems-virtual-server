// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One console's connection, after TLS: its session (`qsisess`), the
//! pairing exchange, each request answered by the families, its
//! streams, and the events it subscribed to. No sockets and no
//! threads: the caller feeds bytes and the time and sends what comes
//! out, so the whole of it runs in a unit test.

mod pair;
pub mod pipe;

use std::collections::BTreeMap;

use qsi::Hello;
use qsi::any::{AnyReply, AnyRequest};
use qsi::host::{console, dumps, vres};
use qsisess::{Incoming, Limits, MAX_ALSO, Note, Role, Session, StreamId};

use crate::events::hub::{self, Shared};
use crate::families::{self, Answer, Open, Services};
use crate::trust::Trust;
use pipe::{Flow, Opener, Pipe};

/// What a connection is: who is at each end.
#[derive(Clone, Debug)]
pub struct Config {
    /// The host's hello.
    pub hello: Hello,
    /// The address the console connected from, for the log.
    pub address: String,
    /// Its certificate, judged by the pins at the handshake.
    pub peer: qsitrust::Peer,
    /// What ties a pairing to this TLS connection.
    pub binding: Option<qsitrust::Binding>,
    /// The session's limits.
    pub limits: Limits,
}

/// How a stream whose far end is done ends.
enum End {
    /// Closed: no more bytes this way.
    Close,
    /// Reset, with why.
    Reset(Note),
}

/// A stream with its far end.
struct Flowing {
    pipe: Box<dyn Pipe>,
    /// Bytes taken from the pipe and not yet sent.
    out: Vec<u8>,
    /// The pipe is done: how the stream ends once `out` is sent.
    end: Option<End>,
}

/// One console's connection.
pub struct Conn {
    session: Session,
    services: Services,
    opener: Box<dyn Opener>,
    hub: Shared,
    trust: Trust,
    config: Config,
    pairing: Option<qsitrust::pair::Managed>,
    streams: BTreeMap<StreamId, Flowing>,
    said: Vec<String>,
    /// Pairing attempts that failed since the last look.
    failed: u32,
    /// How the request being answered went, for its log line.
    outcome: String,
}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn")
            .field("address", &self.config.address)
            .finish_non_exhaustive()
    }
}

fn note(msgid: &str, text: &str) -> Note {
    Note {
        msgid: msgid.into(),
        text: text.into(),
        help: String::new(),
    }
}

impl Conn {
    /// A connection whose TLS handshake is done, at `now_ms`.
    ///
    /// # Errors
    /// The host's hello is out of bounds.
    pub fn new(
        config: Config,
        services: Services,
        opener: Box<dyn Opener>,
        hub: Shared,
        trust: Trust,
        now_ms: u64,
    ) -> Result<Self, qsisess::Error> {
        let session = Session::new(
            Role::Managed,
            config.hello.clone(),
            config.peer.pinned,
            config.limits,
            now_ms,
        )?;
        Ok(Self {
            session,
            services,
            opener,
            hub,
            trust,
            config,
            pairing: None,
            streams: BTreeMap::new(),
            said: Vec::new(),
            failed: 0,
            outcome: String::new(),
        })
    }

    /// Bytes from the connection.
    pub fn feed(&mut self, bytes: &[u8], now_ms: u64) {
        self.session.feed(bytes, now_ms);
    }

    /// One turn: the clock, what the session handed up, the streams'
    /// far ends, the events.
    pub fn step(&mut self, now_ms: u64) {
        self.session.tick(now_ms);
        while let Some(i) = self.session.poll() {
            self.on(i);
        }
        self.pump();
        if self.session.is_open() {
            let hub = hub::lock(&self.hub);
            let _ = self.session.publish(hub.log());
        }
    }

    /// The bytes to send.
    pub fn take_output(&mut self) -> Vec<u8> {
        self.session.take_output()
    }

    /// Whether the session is over.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.session.is_closed()
    }

    /// Close: a goodbye, then nothing more.
    pub fn close(&mut self, why: &str) {
        self.session.close(why);
        self.end_streams();
    }

    /// How many pairing attempts failed since the last call: each
    /// counts against the address as a failed handshake does.
    pub fn failed_pairings(&mut self) -> u32 {
        std::mem::take(&mut self.failed)
    }

    /// The lines for the system log since the last call.
    pub fn said(&mut self) -> Vec<String> {
        std::mem::take(&mut self.said)
    }

    fn say(&mut self, line: String) {
        self.said.push(line);
    }

    fn end_streams(&mut self) {
        for (_, mut f) in std::mem::take(&mut self.streams) {
            f.pipe.reset();
        }
    }

    fn on(&mut self, i: Incoming) {
        match i {
            Incoming::Ready { peer, .. } => {
                let pinned = self.config.peer.pinned;
                self.say(format!(
                    "session with {} {} from {}, {}",
                    peer.name,
                    peer.release,
                    self.config.address,
                    if pinned { "pinned" } else { "to pair" }
                ));
                if pinned {
                    self.trust
                        .contact(&self.config.peer.fingerprint, &self.config.address);
                }
            }
            Incoming::Request(r) => self.request(&r),
            Incoming::Data { stream, bytes } => {
                let Some(f) = self.streams.get_mut(&stream) else {
                    return;
                };
                match f.pipe.put(&bytes) {
                    Ok(()) => {
                        let n = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
                        let _ = self.session.grant(stream, n);
                    }
                    Err(why) => {
                        let _ = self.session.reset_stream(stream, &why.msgid, &why.text);
                        if let Some(mut f) = self.streams.remove(&stream) {
                            f.pipe.reset();
                        }
                    }
                }
            }
            Incoming::StreamClosed { stream } => {
                if let Some(f) = self.streams.get_mut(&stream) {
                    f.pipe.closed();
                }
            }
            Incoming::StreamReset { stream, .. } => {
                if let Some(mut f) = self.streams.remove(&stream) {
                    f.pipe.reset();
                }
            }
            Incoming::Closed { msgid, text, .. } => {
                self.say(format!(
                    "session from {} ended: {} {}",
                    self.config.address,
                    msgid.unwrap_or_default(),
                    text
                ));
                self.end_streams();
            }
            Incoming::Reply(_) | Incoming::Event(_) | Incoming::Writable { .. } => {}
        }
    }

    /// A request: answered, then logged with how it went. The user a
    /// console names is for the log; it grants nothing.
    fn request(&mut self, r: &qsisess::Request) {
        self.outcome = "answered".into();
        self.handle(r);
        let outcome = std::mem::take(&mut self.outcome);
        self.say(format!(
            "request {:04x}/{:04x} from {} by {:?}: {outcome}",
            r.family, r.op, self.config.address, r.user
        ));
    }

    fn handle(&mut self, r: &qsisess::Request) {
        let id = r.id;
        let decoded = match r.decode() {
            Ok(Some(d)) => d,
            Ok(None) => {
                self.outcome = format!("refused {}", qsi::msgid::UNKNOWN);
                let _ = self.session.refuse_unknown(id);
                return;
            }
            Err(e) => {
                self.outcome = format!("refused {}", qsi::msgid::BAD);
                let _ = self.session.refuse_bad(id, &e);
                return;
            }
        };
        if let Err(why) = decoded.check() {
            self.outcome = format!("refused {}: {why}", qsi::msgid::BAD);
            let _ = self.session.refuse(id, qsi::msgid::BAD, why);
            return;
        }
        match decoded {
            AnyRequest::Pair(p) => self.pair(id, p),
            AnyRequest::Base(qsi::base::Request::Subscribe(s)) => {
                let hub = hub::lock(&self.hub);
                let _ = self.session.subscribe(id, hub.log(), s.since);
            }
            AnyRequest::Base(qsi::base::Request::Unsubscribe(_)) => {
                let _ = self.session.unsubscribe(id);
            }
            other => {
                let answer = families::answer(&mut self.services, other, &r.user);
                self.answered(id, answer);
            }
        }
    }

    fn refuse(&mut self, id: u32, notes: &[Note]) {
        let main = notes
            .first()
            .cloned()
            .unwrap_or_else(|| note(qsi::msgid::UNKNOWN, "The request was refused."));
        let also: Vec<Note> = notes.iter().skip(1).take(MAX_ALSO).cloned().collect();
        let ids: Vec<&str> = std::iter::once(main.msgid.as_str())
            .chain(also.iter().map(|n| n.msgid.as_str()))
            .collect();
        self.outcome = format!("refused {}", ids.join(" "));
        let _ = self.session.refuse_with(id, &main, &also);
    }

    fn answered(&mut self, id: u32, answer: Answer) {
        match answer {
            Answer::Reply(reply) => {
                let _ = self.session.reply_any(id, &reply);
            }
            Answer::Refused(notes) => self.refuse(id, &notes),
            Answer::Open(open) => self.open(id, open),
        }
    }

    fn open(&mut self, id: u32, open: Open) {
        let opened = match open {
            Open::Console { id: p, take_over } => {
                self.opener.console(p, take_over).map(|(serial, pipe)| {
                    let reply =
                        AnyReply::Console(console::Reply::Attach(console::Attached { serial }));
                    (reply, pipe)
                })
            }
            Open::Dump { id: p, name } => self.opener.dump(p, &name).map(|(bytes, pipe)| {
                let reply =
                    AnyReply::Dumps(dumps::Reply::Download(dumps::Downloading { name, bytes }));
                (reply, pipe)
            }),
            Open::Upload { name, bytes } => self.opener.upload(&name, bytes).map(|pipe| {
                let reply =
                    AnyReply::Vres(vres::Reply::ImageUpload(vres::Uploading { name, bytes }));
                (reply, pipe)
            }),
        };
        match opened {
            Ok((reply, mut pipe)) => match self.session.reply_stream(id, reply.encode()) {
                Ok(stream) => {
                    self.outcome = format!("stream {stream} opened");
                    self.streams.insert(
                        stream,
                        Flowing {
                            pipe,
                            out: Vec::new(),
                            end: None,
                        },
                    );
                }
                // Refused by the session itself (too many streams).
                Err(_) => pipe.reset(),
            },
            Err(notes) => self.refuse(id, &notes),
        }
    }

    /// Move each stream's bytes as its credit allows; end those whose
    /// far end is done.
    fn pump(&mut self) {
        let mut over = Vec::new();
        for (&stream, f) in &mut self.streams {
            if f.out.is_empty() && f.end.is_none() {
                match f.pipe.poll() {
                    Flow::Bytes(b) => f.out = b,
                    Flow::Idle => {}
                    Flow::End(None) => f.end = Some(End::Close),
                    Flow::End(Some(why)) => f.end = Some(End::Reset(why)),
                }
            }
            if !f.out.is_empty() {
                match self.session.send(stream, &f.out) {
                    Ok(n) => {
                        f.out.drain(..n.min(f.out.len()));
                    }
                    Err(_) => over.push((stream, None)),
                }
            }
            if f.out.is_empty()
                && let Some(why) = f.end.take()
            {
                over.push((stream, Some(why)));
            }
        }
        for (stream, how) in over {
            match how {
                Some(End::Close) => {
                    let _ = self.session.close_stream(stream);
                }
                Some(End::Reset(why)) => {
                    let _ = self.session.reset_stream(stream, &why.msgid, &why.text);
                }
                None => {}
            }
            if let Some(mut f) = self.streams.remove(&stream) {
                f.pipe.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests;
