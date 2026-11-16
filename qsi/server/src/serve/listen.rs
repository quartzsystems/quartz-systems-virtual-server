// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The listener: TCP on each IPv4 management address, following the
//! management network's changes; each connection admitted within the
//! limits, its TLS handshake done within a short time, then its session
//! on a thread of its own.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use rustls::{ServerConnection, StreamOwned};

use super::Server;
use crate::conn::{Config, Conn};

/// The most sessions at once: two consoles, each reconnecting while its
/// old session times out, and pairing.
pub const MAX_SESSIONS: usize = 6;
/// The most handshakes under way at once.
pub const MAX_HANDSHAKES: usize = 4;
/// How long a connection has for its handshake.
pub const HANDSHAKE: Duration = Duration::from_secs(10);
/// Failed handshakes and pairings from one address before it is
/// turned away: more than one password's attempts, so an operator who
/// mistypes can use them all.
pub const MAX_FAILURES: u32 = 10;
/// How long failures are counted, and an address turned away.
pub const FAILURE_WINDOW: Duration = Duration::from_secs(60);
/// How long a session's loop waits for bytes.
const TURN: Duration = Duration::from_millis(20);
/// How often the listener looks at the management addresses.
const LOOK: Duration = Duration::from_secs(5);

/// The counts every connection is admitted by.
#[derive(Debug, Default)]
pub struct Gate {
    sessions: usize,
    handshakes: usize,
    failures: BTreeMap<IpAddr, (u32, Instant)>,
}

/// Why a connection was turned away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turned {
    /// As many sessions as there may be.
    Full,
    /// As many handshakes under way as there may be.
    Busy,
    /// Its address failed too often of late.
    Failing,
}

impl Gate {
    /// Admit a connection from `ip` to its handshake.
    ///
    /// # Errors
    /// Why it is turned away.
    pub fn admit(&mut self, ip: IpAddr, now: Instant) -> Result<(), Turned> {
        self.failures
            .retain(|_, (_, since)| now.duration_since(*since) < FAILURE_WINDOW);
        if self
            .failures
            .get(&ip)
            .is_some_and(|(n, _)| *n >= MAX_FAILURES)
        {
            return Err(Turned::Failing);
        }
        if self.sessions + self.handshakes >= MAX_SESSIONS {
            return Err(Turned::Full);
        }
        if self.handshakes >= MAX_HANDSHAKES {
            return Err(Turned::Busy);
        }
        self.handshakes += 1;
        Ok(())
    }

    /// The handshake failed.
    pub fn failed(&mut self, ip: IpAddr, now: Instant) {
        self.handshakes = self.handshakes.saturating_sub(1);
        let entry = self.failures.entry(ip).or_insert((0, now));
        entry.0 += 1;
    }

    /// A pairing attempt from `ip` failed.
    pub fn failed_pairing(&mut self, ip: IpAddr, now: Instant) {
        let entry = self.failures.entry(ip).or_insert((0, now));
        entry.0 += 1;
    }

    /// The handshake is done: a session.
    pub fn opened(&mut self) {
        self.handshakes = self.handshakes.saturating_sub(1);
        self.sessions += 1;
    }

    /// A session ended.
    pub fn closed(&mut self) {
        self.sessions = self.sessions.saturating_sub(1);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The IPv4 management address in PID 1's `Management` fact
/// (`10.0.2.15/24 on eth0, gateway 10.0.2.2`); none when it has none.
#[must_use]
pub fn management_address(fact: &str) -> Option<Ipv4Addr> {
    let first = fact.split_whitespace().next()?;
    first.split('/').next()?.parse().ok()
}

fn now_ms(t0: Instant) -> u64 {
    u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn quiet(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

/// Finish the handshake within [`HANDSHAKE`].
fn handshake(tls: &mut StreamOwned<ServerConnection, TcpStream>) -> Result<(), String> {
    let until = Instant::now() + HANDSHAKE;
    while tls.conn.is_handshaking() {
        match tls.conn.complete_io(&mut tls.sock) {
            Ok(_) => {}
            Err(e) if quiet(&e) && Instant::now() < until => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

/// One connection, from its handshake to its end.
pub(super) fn connection(server: &Arc<Server>, stream: TcpStream, from: SocketAddr) {
    let ip = from.ip();
    let fail = |why: &str| {
        lock(&server.gate).failed(ip, Instant::now());
        server.say(&format!(
            "a connection from {from} failed its handshake: {why}"
        ));
    };
    if stream.set_read_timeout(Some(TURN)).is_err() || stream.set_nodelay(true).is_err() {
        fail("the socket");
        return;
    }
    let Ok(conn) = ServerConnection::new(server.tls.clone()) else {
        fail("no TLS");
        return;
    };
    let mut tls = StreamOwned::new(conn, stream);
    if let Err(e) = handshake(&mut tls) {
        fail(&e);
        return;
    }
    let Some(peer) = qsitrust::tls::peer(&tls.conn, &server.trust) else {
        fail("no certificate");
        return;
    };
    lock(&server.gate).opened();
    let binding = qsitrust::tls::binding(&tls.conn, &server.identity).ok();
    let config = Config {
        hello: server.hello(),
        address: from.to_string(),
        peer: peer.clone(),
        binding,
        limits: server.limits,
    };
    let t0 = Instant::now();
    let registered = server
        .sessions
        .open(&from.to_string(), &peer, &server.trust);
    let conn = Conn::new(
        config,
        server.services(),
        Box::new(server.opener()),
        server.hub.clone(),
        server.trust.clone(),
        0,
    );
    if let Ok(mut conn) = conn {
        serve(server, &mut conn, &mut tls, t0, &peer, ip);
    }
    server.sessions.close(registered);
    lock(&server.gate).closed();
}

/// The session's loop: bytes in, a turn, bytes out.
fn serve(
    server: &Arc<Server>,
    conn: &mut Conn,
    tls: &mut StreamOwned<ServerConnection, TcpStream>,
    t0: Instant,
    peer: &qsitrust::Peer,
    ip: IpAddr,
) {
    let mut buf = vec![0u8; 64 * 1024];
    let mut checked = Instant::now();
    loop {
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => conn.feed(&buf[..n], now_ms(t0)),
            Err(e) if quiet(&e) => {}
            Err(_) => break,
        }
        conn.step(now_ms(t0));
        // A pin removed at the host ends its session.
        if peer.pinned && checked.elapsed() >= Duration::from_secs(1) {
            checked = Instant::now();
            if !qsitrust::Pins::pinned(&server.trust, &peer.fingerprint) {
                conn.close("the console was removed at the host");
            }
        }
        for line in conn.said() {
            server.say(&line);
        }
        for _ in 0..conn.failed_pairings() {
            lock(&server.gate).failed_pairing(ip, Instant::now());
        }
        let out = conn.take_output();
        if !out.is_empty() && (tls.write_all(&out).is_err() || tls.flush().is_err()) {
            break;
        }
        if conn.is_closed() {
            break;
        }
    }
    tls.conn.send_close_notify();
    let _ = tls.flush();
}

/// Accept on one address until it is no longer a management address.
fn accept(server: &Arc<Server>, listener: &TcpListener, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, from)) => {
                let admitted = lock(&server.gate).admit(from.ip(), Instant::now());
                match admitted {
                    Ok(()) => {
                        let _ = stream.set_nonblocking(false);
                        let s = server.clone();
                        let spawned = thread::Builder::new()
                            .name("session".into())
                            .spawn(move || connection(&s, stream, from));
                        if spawned.is_err() {
                            lock(&server.gate).failed(from.ip(), Instant::now());
                        }
                    }
                    Err(why) => {
                        server.say(&format!("a connection from {from} turned away: {why:?}"));
                    }
                }
            }
            Err(e) if quiet(&e) => thread::sleep(Duration::from_millis(200)),
            Err(_) => thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Listen on each management address as it comes and goes; never
/// returns.
pub fn listen(server: &Arc<Server>) -> ! {
    let mut on: BTreeMap<Ipv4Addr, Arc<AtomicBool>> = BTreeMap::new();
    // The first look, and whether it has listened anywhere yet.
    let (mut first, mut started) = (true, false);
    loop {
        let wanted: Vec<Ipv4Addr> = server.management().into_iter().collect();
        let before: Vec<Ipv4Addr> = on.keys().copied().collect();
        for gone in before.iter().filter(|a| !wanted.contains(a)) {
            if let Some(stop) = on.remove(gone) {
                stop.store(true, Ordering::Relaxed);
            }
        }
        let new: Vec<Ipv4Addr> = wanted
            .iter()
            .filter(|a| !on.contains_key(a))
            .copied()
            .collect();
        for address in new {
            let bound = TcpListener::bind((address, server.port))
                .and_then(|l| l.set_nonblocking(true).map(|()| l));
            match bound {
                Ok(listener) => {
                    let stop = Arc::new(AtomicBool::new(false));
                    on.insert(address, stop.clone());
                    let s = server.clone();
                    let _ = thread::Builder::new()
                        .name("listen".into())
                        .spawn(move || accept(&s, &listener, &stop));
                }
                Err(e) => server.say(&format!("{address}:{}: {e}", server.port)),
            }
        }
        let now: Vec<Ipv4Addr> = on.keys().copied().collect();
        if now != before || first {
            server.listening(&now, !started);
            started |= !now.is_empty();
            first = false;
        }
        thread::sleep(LOOK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_that_fails_is_turned_away_for_a_while() {
        let mut g = Gate::default();
        let ip: IpAddr = "192.0.2.7".parse().unwrap();
        let other: IpAddr = "192.0.2.8".parse().unwrap();
        let t = Instant::now();
        for _ in 0..MAX_FAILURES {
            g.admit(ip, t).unwrap();
            g.failed(ip, t);
        }
        assert_eq!(g.admit(ip, t), Err(Turned::Failing));
        assert_eq!(g.admit(other, t), Ok(()));
        assert_eq!(g.admit(ip, t + FAILURE_WINDOW), Ok(()));
    }

    #[test]
    fn sessions_and_handshakes_are_bounded() {
        let mut g = Gate::default();
        let ip: IpAddr = "192.0.2.7".parse().unwrap();
        let t = Instant::now();
        for _ in 0..MAX_HANDSHAKES {
            g.admit(ip, t).unwrap();
        }
        assert_eq!(g.admit(ip, t), Err(Turned::Busy));
        for _ in 0..MAX_HANDSHAKES {
            g.opened();
        }
        for _ in MAX_HANDSHAKES..MAX_SESSIONS {
            g.admit(ip, t).unwrap();
            g.opened();
        }
        assert_eq!(g.admit(ip, t), Err(Turned::Full));
        g.closed();
        assert_eq!(g.admit(ip, t), Ok(()));
    }

    #[test]
    fn the_management_address_is_read_from_pid_1s_fact() {
        assert_eq!(
            management_address("10.0.2.15/24 on eth0, gateway 10.0.2.2"),
            Some(Ipv4Addr::new(10, 0, 2, 15))
        );
        assert_eq!(
            management_address("none: the network service has not started"),
            None
        );
    }
}
