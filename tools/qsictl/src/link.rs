// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One connection to a host: TCP, TLS 1.3 with the console's identity
//! and the host's pin (or, pairing, any host), and a console's QSI
//! session over it.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use qsi::any::AnyRequest;
use qsi::{Hello, PeerKind, caps};
use qsisess::{Incoming, Limits, Reply, Role, Session, StreamId};
use qsitrust::{Fingerprint, Identity, Pins, tls};
use rustls::{ClientConnection, StreamOwned};

/// How long a request waits for its reply.
const WAIT: Duration = Duration::from_secs(60);

/// What the console accepts of a host's certificate.
#[derive(Debug)]
struct Accept {
    pin: Option<Fingerprint>,
}

impl Pins for Accept {
    fn pinned(&self, peer: &Fingerprint) -> bool {
        self.pin.as_ref() == Some(peer)
    }
    fn pairing_open(&self) -> bool {
        self.pin.is_none()
    }
}

/// A session with a host.
pub struct Link {
    /// The session.
    pub session: Session,
    io: StreamOwned<ClientConnection, TcpStream>,
    t0: Instant,
    /// What came that was not waited for.
    pub besides: Vec<Incoming>,
    /// The user named in each request, for the host's log.
    pub user: String,
    /// Why the session closed, once it has: who closed it and what it
    /// said.
    pub closed: Option<String>,
}

fn quiet(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

impl Link {
    /// Connect to `address` as `identity`, accepting the host pinned as
    /// `pin`, or, with none, any host (to pair).
    ///
    /// # Errors
    /// No connection, no handshake, or no hello.
    pub fn connect(
        address: &str,
        identity: &Identity,
        pin: Option<Fingerprint>,
    ) -> Result<Self, String> {
        let to = address
            .to_socket_addrs()
            .map_err(|e| format!("{address}: {e}"))?
            .next()
            .ok_or_else(|| format!("{address}: no address"))?;
        let sock = TcpStream::connect_timeout(&to, Duration::from_secs(10))
            .map_err(|e| format!("{address}: {e}"))?;
        sock.set_read_timeout(Some(Duration::from_millis(10)))
            .map_err(|e| e.to_string())?;
        let config =
            tls::client_config(identity, Arc::new(Accept { pin })).map_err(|e| e.to_string())?;
        let conn = ClientConnection::new(config, tls::server_name()).map_err(|e| e.to_string())?;
        let mut io = StreamOwned::new(conn, sock);
        let until = Instant::now() + Duration::from_secs(15);
        while io.conn.is_handshaking() {
            match io.conn.complete_io(&mut io.sock) {
                Ok(_) => {}
                Err(e) if quiet(&e) && Instant::now() < until => {}
                Err(e) => return Err(format!("TLS with {address}: {e}")),
            }
        }
        let hello = Hello {
            version: qsi::VERSION,
            caps: caps::HOST_V1 | caps::SERVICING | caps::PHYSICAL_IO | caps::PARTITION_V2,
            kind: PeerKind::Console,
            name: "qsictl".into(),
            release: env!("CARGO_PKG_VERSION").into(),
            serial: String::new(),
        };
        let session = Session::new(Role::Console, hello, true, Limits::default(), 0)
            .map_err(|e| e.to_string())?;
        let mut link = Self {
            session,
            io,
            t0: Instant::now(),
            besides: Vec::new(),
            closed: None,
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "qsictl".into()),
        };
        let until = Instant::now() + Duration::from_secs(15);
        while !link.session.is_open() {
            if link.session.is_closed() || Instant::now() > until {
                return Err(format!("{address} said no hello"));
            }
            let got = link.step()?;
            link.besides.extend(got);
        }
        Ok(link)
    }

    /// The host's certificate.
    #[must_use]
    pub fn peer(&self) -> Option<Fingerprint> {
        self.io
            .conn
            .peer_certificates()
            .and_then(|c| c.first())
            .map(|c| Fingerprint::of(c))
    }

    /// What ties a pairing to this connection.
    ///
    /// # Errors
    /// The connection has none.
    pub fn binding(&self, identity: &Identity) -> Result<qsitrust::Binding, String> {
        tls::binding(&self.io.conn, identity).map_err(|e| e.to_string())
    }

    /// One turn: what is queued out, what came in.
    ///
    /// # Errors
    /// The connection ended.
    pub fn step(&mut self) -> Result<Vec<Incoming>, String> {
        let now = u64::try_from(self.t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        let out = self.session.take_output();
        if !out.is_empty() {
            self.io.write_all(&out).map_err(|e| e.to_string())?;
            self.io.flush().map_err(|e| e.to_string())?;
        }
        let mut buf = vec![0u8; 64 * 1024];
        match self.io.read(&mut buf) {
            Ok(0) => return Err("the host closed the connection".into()),
            Ok(n) => self.session.feed(&buf[..n], now),
            Err(e) if quiet(&e) => {}
            Err(e) => return Err(e.to_string()),
        }
        self.session.tick(now);
        let got: Vec<Incoming> = std::iter::from_fn(|| self.session.poll()).collect();
        for i in &got {
            if let Incoming::Closed {
                msgid,
                text,
                by_peer,
            } = i
            {
                let who = if *by_peer { "the host" } else { "this side" };
                let id = msgid.as_deref().unwrap_or("cleanly");
                self.closed = Some(format!("closed by {who} ({id}): {text}"));
            }
        }
        Ok(got)
    }

    /// An error of the session, with why it closed when it has.
    fn said(&self, e: &qsisess::Error) -> String {
        match &self.closed {
            Some(why) => format!("{e}: {why}"),
            None => e.to_string(),
        }
    }

    /// Ask, and wait for the reply.
    ///
    /// # Errors
    /// The session would not send it, or no reply came.
    pub fn ask(&mut self, r: &AnyRequest) -> Result<Reply, String> {
        let user = self.user.clone();
        let id = self
            .session
            .request_any(&user, r)
            .map_err(|e| self.said(&e))?;
        let until = Instant::now() + WAIT;
        while Instant::now() < until {
            let mut reply = None;
            for i in self.step()? {
                match i {
                    Incoming::Reply(r) if r.id == id => reply = Some(r),
                    other => self.besides.push(other),
                }
            }
            if let Some(r) = reply {
                return Ok(r);
            }
        }
        Err("no reply in time".into())
    }

    /// Send all of `bytes` on `stream`, as its credit allows.
    ///
    /// # Errors
    /// The stream or the connection ended.
    pub fn send_all(&mut self, stream: StreamId, mut bytes: &[u8]) -> Result<(), String> {
        while !bytes.is_empty() {
            let n = self
                .session
                .send(stream, bytes)
                .map_err(|e| self.said(&e))?;
            bytes = &bytes[n..];
            if n == 0 {
                let got = self.step()?;
                for i in &got {
                    if let Incoming::StreamReset {
                        stream: s,
                        msgid,
                        text,
                        ..
                    } = i
                        && *s == stream
                    {
                        return Err(format!("{msgid} {text}"));
                    }
                }
                self.besides.extend(got);
            }
        }
        Ok(())
    }

    /// Write `bytes` into the TLS connection as they are, past the
    /// session: for a harness that sends what no session would.
    ///
    /// # Errors
    /// The connection ended.
    pub fn send_raw(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.io.write_all(bytes).map_err(|e| e.to_string())?;
        self.io.flush().map_err(|e| e.to_string())
    }

    /// Close cleanly.
    pub fn close(&mut self) {
        self.session.close("done");
        let _ = self.step();
        self.io.conn.send_close_notify();
        let _ = self.io.flush();
    }
}
