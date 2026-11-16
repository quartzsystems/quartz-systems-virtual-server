// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The server over real TLS on loopback: a console pairs with the
//! password out, connects again pinned and is answered; one that is
//! not pinned once pairing is over gets no session.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qsi::any::{AnyReply, AnyRequest};
use qsi::{Empty, Hello, PeerKind, base, caps, pair};
use qsisess::{Incoming, Limits, Reply, Role, Session};
use qsitrust::{Fingerprint, Identity, Pins, tls};
use rustls::{ClientConnection, StreamOwned};

use super::facts::Paths;
use super::{Server, Settings, listen};
use crate::trust::Trust;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("qsi-serve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn settings(d: &std::path::Path) -> Settings {
    Settings {
        paths: Paths {
            state: d.join("state"),
            log: d.join("log"),
            control: d.join("control.sock"),
            dumps: d.join("dumps"),
        },
        own: d.to_path_buf(),
        local: d.join("local.sock"),
        partd: d.join("partd.sock"),
        poold: d.join("poold.sock"),
        vswitchd: d.join("vswitchd.sock"),
        probd: d.join("probd.sock"),
        port: 0,
        release: "V1R0M0".into(),
        build: "test".into(),
        limits: Limits::default(),
        pairing_secs: crate::trust::LIFETIME_SECS,
    }
}

/// The console's pins: the host once paired, any while pairing.
#[derive(Debug, Default, Clone)]
struct ConsolePins(Arc<Mutex<(Vec<Fingerprint>, bool)>>);

impl Pins for ConsolePins {
    fn pinned(&self, peer: &Fingerprint) -> bool {
        self.0.lock().unwrap().0.contains(peer)
    }
    fn pairing_open(&self) -> bool {
        self.0.lock().unwrap().1
    }
}

struct Console {
    session: Session,
    io: StreamOwned<ClientConnection, TcpStream>,
    t0: Instant,
}

impl Console {
    fn connect(server: &Arc<Server>, id: &Identity, pins: &ConsolePins) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let s = server.clone();
        std::thread::spawn(move || {
            let (stream, from) = listener.accept().unwrap();
            listen::connection(&s, stream, from);
        });
        let sock = TcpStream::connect(address).map_err(|e| e.to_string())?;
        sock.set_read_timeout(Some(Duration::from_millis(5)))
            .unwrap();
        let config = tls::client_config(id, Arc::new(pins.clone())).unwrap();
        let conn = ClientConnection::new(config, tls::server_name()).unwrap();
        let mut io = StreamOwned::new(conn, sock);
        let until = Instant::now() + Duration::from_secs(10);
        while io.conn.is_handshaking() {
            match io.conn.complete_io(&mut io.sock) {
                Ok(_) => {}
                Err(e)
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                        && Instant::now() < until => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        let hello = Hello {
            version: qsi::VERSION,
            caps: caps::HOST_V1,
            kind: PeerKind::Console,
            name: "MC1".into(),
            release: "V1R0M0".into(),
            serial: String::new(),
        };
        let session = Session::new(Role::Console, hello, true, Limits::default(), 0).unwrap();
        let mut c = Self {
            session,
            io,
            t0: Instant::now(),
        };
        let until = Instant::now() + Duration::from_secs(10);
        while !c.session.is_open() {
            if c.session.is_closed() || Instant::now() > until {
                return Err("no session".into());
            }
            if c.step().is_err() {
                return Err("the connection ended".into());
            }
        }
        Ok(c)
    }

    fn step(&mut self) -> Result<Vec<Incoming>, ()> {
        let now = u64::try_from(self.t0.elapsed().as_millis()).unwrap();
        let out = self.session.take_output();
        if !out.is_empty() {
            self.io.write_all(&out).map_err(|_| ())?;
            self.io.flush().map_err(|_| ())?;
        }
        let mut buf = [0u8; 16384];
        match self.io.read(&mut buf) {
            Ok(0) => return Err(()),
            Ok(n) => self.session.feed(&buf[..n], now),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return Err(()),
        }
        self.session.tick(now);
        Ok(std::iter::from_fn(|| self.session.poll()).collect())
    }

    fn ask(&mut self, r: &AnyRequest) -> Reply {
        let id = self.session.request_any("QSECOFR", r).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            for i in self.step().expect("the connection") {
                if let Incoming::Reply(r) = i
                    && r.id == id
                {
                    return r;
                }
            }
        }
        panic!("no reply");
    }
}

#[test]
fn a_console_pairs_over_tls_and_is_answered_pinned() {
    let d = dir("pair");
    let host = Identity::make("HOST1", "S1", 1_794_488_700).unwrap();
    let console = Identity::make("MC1", "", 1_794_488_700).unwrap();
    let server = Server::new(settings(&d), host.clone()).unwrap();
    let password = server.trust.start_pairing().unwrap();
    let pins = ConsolePins::default();
    pins.0.lock().unwrap().1 = true;

    let mut c = Console::connect(&server, &console, &pins).unwrap();
    let refused = c.ask(&AnyRequest::Base(base::Request::Identify(Empty)));
    assert_eq!(
        refused.refusal().map(|(m, _)| m),
        Some(qsi::msgid::NOT_PINNED)
    );
    let binding = tls::binding(&c.io.conn, &console).unwrap();
    let (exchange, start) = qsitrust::pair::Console::start(&password.text, binding, "MC1").unwrap();
    let reply = c.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: start.spake,
        name: start.name,
    })));
    let Ok(Some(pair::Reply::Answer(a))) = reply.typed::<pair::Reply>() else {
        panic!("{:?}", reply.refusal());
    };
    let (confirm, paired) = exchange
        .answer(&qsitrust::pair::Answer {
            spake: a.spake,
            confirm: a.confirm,
            name: a.name,
        })
        .unwrap();
    assert_eq!(paired.fingerprint, host.fingerprint());
    let done = c.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
        confirm: confirm.confirm,
    })));
    assert_eq!(done.refusal(), None);
    {
        let mut p = pins.0.lock().unwrap();
        p.0.push(paired.fingerprint);
        p.1 = false;
    }
    drop(c);

    // Pinned both ways: answered. None of the host's state files is
    // there, so who it is is little more than its kind and release.
    let mut c = Console::connect(&server, &console, &pins).unwrap();
    let reply = c.ask(&AnyRequest::Base(base::Request::Identify(Empty)));
    let Ok(Some(AnyReply::Base(base::Reply::Identify(me)))) = reply.decode() else {
        panic!("{:?}", reply.refusal());
    };
    assert_eq!((me.kind, me.release.as_str()), (PeerKind::Host, "V1R0M0"));
    // The pin was kept on disk.
    let kept = Trust::open(d.clone(), Arc::new(|| 0)).unwrap();
    assert_eq!(kept.consoles().consoles.len(), 1);
    drop(c);

    // A console nobody pinned, with no password out: no session.
    let stranger = Identity::make("MC2", "", 1_794_488_700).unwrap();
    let open = ConsolePins::default();
    open.0.lock().unwrap().1 = true;
    assert!(Console::connect(&server, &stranger, &open).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}
