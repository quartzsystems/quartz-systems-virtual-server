// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use std::sync::{Arc, Mutex};

use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::host::{dumps, partitions};
use qsi::{Empty, Hello, PeerKind, base, caps, pair};
use qsisess::{Incoming, Limits, Role, Session};
use qsitrust::{Binding, Fingerprint, Peer};

use super::pipe::{Flow, Opener, Pipe};
use super::{Config, Conn};
use crate::events::Hub;
use crate::families::{Facts, Services};
use crate::trust::{Consoles, Trust};

fn hello(kind: PeerKind, name: &str) -> Hello {
    Hello {
        version: qsi::VERSION,
        caps: caps::HOST_V1,
        kind,
        name: name.into(),
        release: "V1R0M0".into(),
        serial: String::new(),
    }
}

struct Gone;

fn gone() -> ipc::WireError {
    ipc::WireError::Io(std::io::Error::other("no socket"))
}

impl partd::Calls for Gone {
    fn call(&mut self, req: &partd::Request) -> Result<Vec<u8>, ipc::WireError> {
        match req {
            partd::Request::List {} => Ok(ipc::encode(&Vec::<partd::Status>::new())),
            _ => Err(gone()),
        }
    }
}
impl poold::Calls for Gone {
    fn call(&mut self, _: &poold::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl problem::proto::Calls for Gone {
    fn call(&mut self, _: &problem::proto::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl update::proto::Calls for Gone {
    fn call(&mut self, _: &update::proto::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl vswitchd::Calls for Gone {
    fn call(&mut self, _: &vswitchd::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl Facts for Gone {
    fn host_dumps(&mut self) -> Vec<hostdump::Info> {
        Vec::new()
    }
    fn take_host_dump(&mut self, _: &str) -> Result<String, Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn delete_host_dump(&mut self, _: &str) -> Result<(), Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn inventory(&mut self) -> Result<state::Inventory, Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn identity(&mut self) -> Result<base::Identity, Vec<qsisess::Note>> {
        Ok(base::Identity {
            kind: PeerKind::Host,
            name: "HOST1".into(),
            release: "V1R0M0".into(),
            serial: "S1".into(),
            model: String::new(),
            build: String::new(),
        })
    }
    fn state(&mut self) -> Result<supervise::control::HostState, Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn power(&mut self, _: bool, _: bool, _: u32, _: &str) -> Result<(), Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn history(&mut self) -> Result<Vec<crate::families::base::Posted>, Vec<qsisess::Note>> {
        Ok(Vec::new())
    }
    fn time(&mut self) -> base::Time {
        base::Time {
            unix_ns: 0,
            offset_s: 0,
            zone: "UTC".into(),
        }
    }
}

/// A dump of a few pieces, and what it was told.
struct Pieces {
    left: Vec<Vec<u8>>,
    reset: Arc<Mutex<bool>>,
}

impl Pipe for Pieces {
    fn poll(&mut self) -> Flow {
        if self.left.is_empty() {
            Flow::End(None)
        } else {
            Flow::Bytes(self.left.remove(0))
        }
    }
    fn put(&mut self, _: &[u8]) -> Result<(), qsisess::Note> {
        Ok(())
    }
    fn closed(&mut self) {}
    fn reset(&mut self) {
        *self.reset.lock().unwrap() = true;
    }
}

#[derive(Default)]
struct Opens {
    reset: Arc<Mutex<bool>>,
}

impl Opener for Opens {
    fn stage(&mut self, _: &str, _: u64, _: bool) -> Result<Box<dyn Pipe>, Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn host_dump(&mut self, _: &str) -> Result<(u64, Box<dyn Pipe>), Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn console(&mut self, _: u32, _: bool) -> Result<(bool, Box<dyn Pipe>), Vec<qsisess::Note>> {
        Err(Vec::new())
    }
    fn dump(&mut self, _: u32, _: &str) -> Result<(u64, Box<dyn Pipe>), Vec<qsisess::Note>> {
        Ok((
            6,
            Box::new(Pieces {
                left: vec![b"ELF".to_vec(), b"abc".to_vec()],
                reset: self.reset.clone(),
            }),
        ))
    }
    fn upload(&mut self, _: &str, _: u64) -> Result<Box<dyn Pipe>, Vec<qsisess::Note>> {
        Err(Vec::new())
    }
}

const HOST_PRINT: Fingerprint = Fingerprint([1; 32]);
const CONSOLE_PRINT: Fingerprint = Fingerprint([2; 32]);
const EXPORTER: [u8; 32] = [7; 32];

struct Pair {
    host: Conn,
    console: Session,
    now: u64,
    hub: crate::events::hub::Shared,
    trust: Trust,
    reset: Arc<Mutex<bool>>,
    /// What came with a reply.
    besides: Vec<Incoming>,
}

impl Pair {
    fn new(pinned: bool, trust: Trust) -> Self {
        let hub = Arc::new(Mutex::new(Hub::memory(&Limits::default())));
        let opens = Opens::default();
        let reset = opens.reset.clone();
        let config = Config {
            hello: hello(PeerKind::Host, "HOST1"),
            address: "192.0.2.9:50000".into(),
            peer: Peer {
                cert: Vec::new(),
                fingerprint: CONSOLE_PRINT,
                pinned,
            },
            binding: Some(Binding {
                mine: HOST_PRINT,
                peer: CONSOLE_PRINT,
                exporter: EXPORTER,
            }),
            limits: Limits::default(),
        };
        let services = Services {
            partd: Box::new(Gone),
            poold: Box::new(Gone),
            vswitchd: Box::new(Gone),
            probd: Box::new(Gone),
            updd: Box::new(Gone),
            facts: Box::new(Gone),
        };
        let host = Conn::new(
            config,
            services,
            Box::new(opens),
            hub.clone(),
            trust.clone(),
            0,
        )
        .unwrap();
        let console = Session::new(
            Role::Console,
            hello(PeerKind::Console, "MC1"),
            true,
            Limits::default(),
            0,
        )
        .unwrap();
        let mut p = Self {
            host,
            console,
            now: 0,
            hub,
            trust,
            reset,
            besides: Vec::new(),
        };
        p.pump();
        p
    }

    /// Move bytes both ways until neither has any; what the console
    /// was handed.
    fn pump(&mut self) -> Vec<Incoming> {
        let mut got = Vec::new();
        for _ in 0..50 {
            self.now += 1;
            self.host.step(self.now);
            let out = self.host.take_output();
            if !out.is_empty() {
                self.console.feed(&out, self.now);
            }
            while let Some(i) = self.console.poll() {
                got.push(i);
            }
            let back = self.console.take_output();
            if out.is_empty() && back.is_empty() {
                break;
            }
            self.host.feed(&back, self.now);
        }
        got
    }

    fn ask(&mut self, r: &AnyRequest) -> qsisess::Reply {
        let id = self.console.request_any("ADMIN", r).unwrap();
        let mut reply = None;
        for i in self.pump() {
            match i {
                Incoming::Reply(r) if r.id == id => reply = Some(r),
                other => self.besides.push(other),
            }
        }
        reply.expect("a reply")
    }
}

fn trust() -> Trust {
    Trust::with(Consoles::default(), None, Arc::new(|| 1000))
}

#[test]
fn a_console_pairs_with_the_password_out_and_nothing_else() {
    let trust = trust();
    let password = trust.start_pairing().unwrap();
    let mut p = Pair::new(false, trust);
    let console_binding = Binding {
        mine: CONSOLE_PRINT,
        peer: HOST_PRINT,
        exporter: EXPORTER,
    };
    let (console, start) =
        qsitrust::pair::Console::start(&password.text, console_binding, "MC1").unwrap();
    let reply = p.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: start.spake,
        name: start.name,
    })));
    let Ok(Some(pair::Reply::Answer(a))) = reply.typed::<pair::Reply>() else {
        panic!("{:?}", reply.refusal());
    };
    let (confirm, host) = console
        .answer(&qsitrust::pair::Answer {
            spake: a.spake,
            confirm: a.confirm,
            name: a.name,
        })
        .unwrap();
    assert_eq!(
        (host.fingerprint, host.name.as_str()),
        (HOST_PRINT, "HOST1")
    );
    let done = p.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
        confirm: confirm.confirm,
    })));
    assert_eq!(done.refusal(), None);
    let consoles = p.trust.consoles();
    assert_eq!(consoles.consoles.len(), 1);
    assert_eq!(consoles.consoles[0].name, "MC1");
    assert!(p.trust.password().is_none(), "good for one pairing");
    // A second start has no password.
    let again = p.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: vec![0; 33],
        name: "MC2".into(),
    })));
    assert_eq!(
        again.refusal().map(|(m, _)| m),
        Some(qsi::msgid::NO_PAIRING)
    );
    assert!(p.host.said().iter().any(|l| l.contains("paired")));
}

#[test]
fn a_wrong_password_pins_nothing_and_counts_an_attempt() {
    let trust = trust();
    trust.start_pairing().unwrap();
    let mut p = Pair::new(false, trust);
    let console_binding = Binding {
        mine: CONSOLE_PRINT,
        peer: HOST_PRINT,
        exporter: EXPORTER,
    };
    let (console, start) =
        qsitrust::pair::Console::start("WRONGPASSWORD000", console_binding, "MC1").unwrap();
    let reply = p.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: start.spake,
        name: start.name,
    })));
    let Ok(Some(pair::Reply::Answer(a))) = reply.typed::<pair::Reply>() else {
        panic!("{:?}", reply.refusal());
    };
    // The console cannot check the host's confirmation; it sends what
    // it has, and the host refuses it.
    let confirm = console.answer(&qsitrust::pair::Answer {
        spake: a.spake,
        confirm: a.confirm,
        name: a.name,
    });
    assert!(confirm.is_err());
    let refused = p.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
        confirm: vec![0; 32],
    })));
    assert_eq!(
        refused.refusal().map(|(m, _)| m),
        Some(qsi::msgid::PAIRING_FAILED)
    );
    assert!(p.trust.consoles().consoles.is_empty());
    assert_eq!(
        p.trust.password().unwrap().attempts,
        crate::trust::ATTEMPTS - 1
    );
}

#[test]
fn an_unpinned_console_is_answered_only_for_pairing() {
    let mut p = Pair::new(false, trust());
    let reply = p.ask(&AnyRequest::Base(base::Request::Identify(Empty)));
    assert_eq!(
        reply.refusal().map(|(m, _)| m),
        Some(qsi::msgid::NOT_PINNED)
    );
}

#[test]
fn a_pinned_console_is_answered_and_given_its_events() {
    let mut p = Pair::new(true, trust());
    let reply = p.ask(&AnyRequest::Base(base::Request::Identify(Empty)));
    let Ok(Some(AnyReply::Base(base::Reply::Identify(me)))) = reply.decode() else {
        panic!("{:?}", reply.refusal());
    };
    assert_eq!(me.name, "HOST1");
    let reply = p.ask(&AnyRequest::Partitions(partitions::Request::List(Empty)));
    assert!(matches!(
        reply.decode(),
        Ok(Some(AnyReply::Partitions(partitions::Reply::List(_))))
    ));
    let reply = p.ask(&AnyRequest::Base(base::Request::Subscribe(
        base::Subscribe { since: None },
    )));
    assert_eq!(reply.refusal(), None);
    let event = AnyEvent::Partitions(partitions::Event::Defined(partitions::Defined {
        id: 1,
        name: "P1".into(),
        exists: true,
        time: 5,
    }));
    crate::events::hub::lock(&p.hub).push(&event).unwrap();
    let got = p.pump();
    let seen: Vec<u64> = got
        .iter()
        .filter_map(|i| match i {
            Incoming::Event(e) => Some(e.seq),
            _ => None,
        })
        .collect();
    assert_eq!(seen, vec![1]);
    // A request a family does not read is refused by name.
    let reply = p.ask(&AnyRequest::Partitions(partitions::Request::Get(
        partitions::Get { id: 0 },
    )));
    assert_eq!(reply.refusal().map(|(m, _)| m), Some(qsi::msgid::BAD));
}

#[test]
fn a_dump_is_streamed_and_its_stream_ends_after_the_last_byte() {
    let mut p = Pair::new(true, trust());
    let reply = p.ask(&AnyRequest::Dumps(dumps::Request::Download(
        dumps::Download {
            id: 1,
            name: "D1".into(),
        },
    )));
    let stream = reply.stream.expect("a stream");
    let mut bytes = Vec::new();
    let mut closed = false;
    let mut got = std::mem::take(&mut p.besides);
    for _ in 0..5 {
        got.extend(p.pump());
    }
    {
        for i in got {
            match i {
                Incoming::Data {
                    stream: s,
                    bytes: b,
                } if s == stream => bytes.extend(b),
                Incoming::StreamClosed { stream: s } if s == stream => closed = true,
                _ => {}
            }
        }
    }
    assert_eq!(bytes, b"ELFabc");
    assert!(closed);
    assert!(*p.reset.lock().unwrap(), "the far end is let go");
}
