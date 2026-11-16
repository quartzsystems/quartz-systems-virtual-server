// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The fuzz target of what a console sends after its handshake: a
//! pinned session's bytes, in pieces of any size, through the session,
//! the requests' checks and their translation, with every service
//! answering that it is not there. Nothing may panic, and the server
//! answers or closes.

use std::sync::{Arc, Mutex};

use qsi::{Hello, PeerKind, base, caps};
use qsisess::{Limits, Note, Role, Session};

use crate::conn::pipe::{Opener, Pipe};
use crate::conn::{Config, Conn};
use crate::events::Hub;
use crate::families::{Facts, Services, down};
use crate::trust::{Consoles, Trust};

struct Nothing;

fn gone() -> ipc::WireError {
    ipc::WireError::Io(std::io::Error::other("no socket"))
}

impl partd::Calls for Nothing {
    fn call(&mut self, _: &partd::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl poold::Calls for Nothing {
    fn call(&mut self, _: &poold::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl problem::proto::Calls for Nothing {
    fn call(&mut self, _: &problem::proto::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl vswitchd::Calls for Nothing {
    fn call(&mut self, _: &vswitchd::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl Facts for Nothing {
    fn inventory(&mut self) -> Result<state::Inventory, Vec<Note>> {
        Err(down("inventory", &gone()))
    }
    fn identity(&mut self) -> Result<base::Identity, Vec<Note>> {
        Err(down("PID 1", &gone()))
    }
    fn state(&mut self) -> Result<supervise::control::HostState, Vec<Note>> {
        Err(down("PID 1", &gone()))
    }
    fn power(&mut self, _: bool, _: bool, _: u32, _: &str) -> Result<(), Vec<Note>> {
        Err(down("PID 1", &gone()))
    }
    fn history(&mut self) -> Result<Vec<crate::families::base::Posted>, Vec<Note>> {
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
impl Opener for Nothing {
    fn console(&mut self, _: u32, _: bool) -> Result<(bool, Box<dyn Pipe>), Vec<Note>> {
        Err(down("partition manager", &gone()))
    }
    fn dump(&mut self, _: u32, _: &str) -> Result<(u64, Box<dyn Pipe>), Vec<Note>> {
        Err(down("partition manager", &gone()))
    }
    fn upload(&mut self, _: &str, _: u64) -> Result<Box<dyn Pipe>, Vec<Note>> {
        Err(down("pool service", &gone()))
    }
}

fn hello(kind: PeerKind) -> Hello {
    Hello {
        version: qsi::VERSION,
        caps: caps::HOST_V1,
        kind,
        name: "FUZZ".into(),
        release: "V1R0M0".into(),
        serial: String::new(),
    }
}

fn conn() -> Conn {
    let config = Config {
        hello: hello(PeerKind::Host),
        address: "192.0.2.1:1".into(),
        peer: qsitrust::Peer {
            cert: Vec::new(),
            fingerprint: qsitrust::Fingerprint([9; 32]),
            pinned: true,
        },
        binding: None,
        limits: Limits::default(),
    };
    let services = Services {
        partd: Box::new(Nothing),
        poold: Box::new(Nothing),
        vswitchd: Box::new(Nothing),
        probd: Box::new(Nothing),
        facts: Box::new(Nothing),
    };
    let hub = Arc::new(Mutex::new(Hub::memory(&Limits::default())));
    let trust = Trust::with(Consoles::default(), None, Arc::new(|| 0));
    // Its own hello is in bounds: this cannot fail.
    Conn::new(config, services, Box::new(Nothing), hub, trust, 0).expect("a connection")
}

/// One input: the first byte chooses the piece size.
pub fn session(data: &[u8]) {
    let Some((&size, rest)) = data.split_first() else {
        return;
    };
    let size = usize::from(size).max(1);
    let mut c = conn();
    let mut now = 0;
    for piece in rest.chunks(size) {
        now += 7;
        c.feed(piece, now);
        c.step(now);
        let _ = c.take_output();
        if c.is_closed() {
            break;
        }
    }
}

/// The seeds: a console's hello and one of every request, as it sends
/// them, in one input.
#[must_use]
pub fn seeds() -> Vec<Vec<u8>> {
    let Ok(mut console) = Session::new(
        Role::Console,
        hello(PeerKind::Console),
        true,
        Limits::default(),
        0,
    ) else {
        return Vec::new();
    };
    let mut first = console.take_output();
    // The host's hello, so the console may ask.
    if let Ok(mut host) = Session::new(
        Role::Managed,
        hello(PeerKind::Host),
        true,
        Limits::default(),
        0,
    ) {
        console.feed(&host.take_output(), 1);
        while console.poll().is_some() {}
    }
    let mut out = Vec::new();
    for (request, _) in qsi::samples::exchanges() {
        if console.request_any("QSECOFR", &request).is_ok() {
            let mut one = vec![61];
            one.extend(&first);
            one.extend(console.take_output());
            out.push(one);
        }
    }
    first.insert(0, 3);
    out.push(first);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_seed_runs() {
        let seeds = seeds();
        assert!(seeds.len() > 50);
        for s in &seeds {
            session(s);
        }
        session(&[]);
        session(&[0, 0xff, 0x10, 0x01]);
    }
}
