// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 11, 12 and 14: hostile peers, the one listening socket, and
//! the request log.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::base;
use qsi::host::{partitions, vres};

use super::families::ask;

/// A TCP socket's state of listening, as `/proc/net/tcp` writes it.
const LISTEN: u8 = 0x0a;
use super::{eventually, host, host_with_pool, link, paired};
use crate::suite::base::{Run, channel};

/// Whether the server ends a connection that says nothing within
/// `within`; how long it took.
fn ended_within(address: &str, send: &[u8], within: Duration) -> Result<Option<Duration>, String> {
    let mut s = TcpStream::connect(address).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|e| e.to_string())?;
    if !send.is_empty() {
        let _ = s.write_all(send);
    }
    let began = Instant::now();
    let mut buf = [0u8; 4096];
    while began.elapsed() < within {
        match s.read(&mut buf) {
            Ok(0) => return Ok(Some(began.elapsed())),
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Ok(Some(began.elapsed())),
        }
    }
    Ok(None)
}

/// A link once the server has let go of the flood's connections: until
/// it sees them closed they hold its four handshakes, and a fifth is
/// ended, as it should be. Tried every 3 s for 30 s.
fn link_when_free(
    q: &super::Qsi<'_>,
    id: &qsitrust::Identity,
    host: qsitrust::Fingerprint,
) -> Result<qsictl::link::Link, String> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        match link(q, id, host) {
            Ok(l) => return Ok(l),
            Err(_) if Instant::now() < until => std::thread::sleep(Duration::from_secs(3)),
            Err(e) => return Err(e),
        }
    }
}

fn qsid_pid(q: &mut super::Qsi<'_>) -> Result<u32, String> {
    Ok(channel(&mut q.guest)?
        .status()?
        .service("qsid")
        .map_or(0, |s| s.pid))
}

/// Suite 11: floods, slow handshakes, nonsense before and after the
/// handshake, a thousand streams and credit never given: the host and
/// its server are unaffected, and the limits answer by name.
pub fn hostile(run: &Run<'_>) -> Result<String, String> {
    let mut q = host_with_pool(run)?;
    let deadline = run.deadline();
    let (id, host_print, mut l) = paired(&mut q, "MC1", deadline)?;
    let pid = qsid_pid(&mut q)?;
    // A flood: many connections that never speak TLS.
    let flood: Vec<TcpStream> = (0..64)
        .filter_map(|_| TcpStream::connect(&q.address).ok())
        .collect();
    let answered = ask(&mut l, AnyRequest::Base(base::Request::Identify(Empty))).is_ok();
    // A slow handshake: nothing said, ended within the handshake's time.
    let slow = ended_within(&q.address, &[], Duration::from_secs(20))?
        .ok_or("a connection that said nothing was not ended")?;
    drop(flood);
    // Nonsense before the handshake.
    let junk = ended_within(
        &q.address,
        &[0x16, 0x03, 0x01, 0xff, 0xff, 0, 1, 2, 3],
        Duration::from_secs(20),
    )?
    .ok_or("nonsense before the handshake did not end the connection")?;
    // After it: a frame longer than any message, on its own link.
    let mut bad = link_when_free(&q, &id, host_print)?;
    let mut frame = vec![0x51, 0x53, 0x49, 0x00, 0x10, 0x10];
    frame.extend(u32::MAX.to_be_bytes());
    frame.extend(vec![0xa5; 4096]);
    bad.send_raw(&frame)?;
    let closed = eventually(Duration::from_secs(20), || bad.step().err())
        .ok_or("a frame past every bound did not end the session")?;
    // Streams past the limit: uploads that never send a byte.
    let mut by_name = Vec::new();
    let mut refused = None;
    for n in 0..1000 {
        let reply = l.ask(&AnyRequest::Vres(vres::Request::ImageUpload(
            vres::ImageUpload {
                name: format!("S{n}"),
                bytes: 1 << 20,
            },
        )))?;
        match reply.refusal() {
            Some((m, t)) => {
                refused = Some(format!("{m} {t}"));
                break;
            }
            None => by_name.push(reply.stream),
        }
    }
    let refused = refused.ok_or("a thousand streams were opened")?;
    // Requests answer beside streams whose credit is never given back.
    let t = Instant::now();
    ask(
        &mut l,
        AnyRequest::Partitions(partitions::Request::List(Empty)),
    )?;
    let took = t.elapsed();
    l.close();
    let mut again = link_when_free(&q, &id, host_print)?;
    let AnyReply::Base(base::Reply::State(s)) =
        ask(&mut again, AnyRequest::Base(base::Request::State(Empty)))?
    else {
        return Err("state".into());
    };
    again.close();
    if qsid_pid(&mut q)? != pid {
        return Err("the server was started again under the hostile peers".into());
    }
    Ok(format!(
        "64 connections that never spoke TLS, identify answered beside them: {answered}; a \
         handshake that said nothing ended after {slow:?}; nonsense before the handshake ended \
         after {junk:?}; a frame past every bound ended its session ({closed}); {} streams \
         opened, then `{refused}`; a list in {took:?} beside them; the host {:?}, the server's \
         process the same",
        by_name.len(),
        s.power
    ))
}

/// Suite 12: the only listening socket on the host is TCP 2300 on the
/// management address.
pub fn listen(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, false)?;
    let mut listening = Vec::new();
    for (file, v6) in [("tcp", false), ("tcp6", true)] {
        let text = channel(&mut q.guest)?.text(&format!("/proc/net/{file}"))?;
        for s in netlink::procnet::parse(&text, v6) {
            if s.state == LISTEN {
                listening.push(s.local);
            }
        }
    }
    let wanted: std::net::SocketAddr = format!("10.0.2.15:{}", release::QSI_PORT)
        .parse()
        .map_err(|e| format!("{e}"))?;
    if listening != [wanted] {
        return Err(format!("listening: {listening:?}"));
    }
    Ok(format!(
        "one listening socket, {wanted}: the QSI host server's; nothing else, on any address"
    ))
}

/// Suite 14: the log has every request with its console and user, and
/// no password, key or screen.
pub fn log(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, false)?;
    let deadline = run.deadline();
    super::to_pairing(&mut q, deadline)?;
    let (password, _) = super::start_pairing(&mut q, deadline)?;
    let id = super::identity("MC1")?;
    let host_print = super::pair(&q, &id, &password, "MC1")?.fingerprint;
    let mut l = link(&q, &id, host_print)?;
    l.user = "OPERATOR7".into();
    for r in [
        AnyRequest::Base(base::Request::Identify(Empty)),
        AnyRequest::Partitions(partitions::Request::List(Empty)),
        AnyRequest::Vres(vres::Request::Pools(Empty)),
    ] {
        ask(&mut l, r)?;
    }
    l.close();
    std::thread::sleep(Duration::from_secs(3));
    let log = channel(&mut q.guest)?.text("/data/log/system.ndjson")?;
    let requests: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("request ") && l.contains("OPERATOR7"))
        .collect();
    if requests.len() < 3 {
        return Err(format!("{} requests logged by OPERATOR7", requests.len()));
    }
    for (what, bad) in [
        ("the pairing password", password.as_str()),
        ("a private key", "PRIVATE KEY"),
        ("a key's bytes", "BEGIN"),
    ] {
        if log.contains(bad) {
            return Err(format!("the log has {what}"));
        }
    }
    if !log.contains("paired") {
        return Err("the pairing is not in the log".into());
    }
    Ok(format!(
        "{} requests logged with the console's address and user OPERATOR7, e.g. `{}`; the \
         pairing logged; no password, key or screen in the log",
        requests.len(),
        requests[0].chars().take(120).collect::<String>()
    ))
}
