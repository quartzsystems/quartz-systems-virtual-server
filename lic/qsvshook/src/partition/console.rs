// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The harness's console viewer and dump reader, through the partition
//! manager's socket as QS/MC's will be: in a test image only.
//!
//! `console <id> <mode> <ms> [hex]`: attach (`view`, or `take` to take
//! it over), send the `ConsoleIn` messages the hex encodes (an `ipc`
//! list) a little apart, watch `ms` milliseconds more, detach. With
//! `second`, a second viewer is tried while this one has it, then one
//! that takes it over. The answer: `fact` lines (`serial`, `frames`,
//! `updates`, `ended`, `second`, `takeover`), then the screen as it was
//! last shown, a `line` a row.
//!
//! `dumpread <id> <name>`: the dump through the socket's streamed read:
//! `ok <bytes> <sum of its bytes>`.

use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use partd::proto::{ConsoleIn, ConsoleOut};
use qsipart::console::{FromSystem, Grid};

use super::relay::client;

/// How far apart the messages go.
const APART: Duration = Duration::from_millis(60);

#[derive(Default)]
struct Seen {
    grid: Grid,
    serial: Vec<u8>,
    frames: u32,
    updates: u32,
    ended: String,
    /// When each frame or update came.
    came: Vec<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn refused(errors: &[qvsmsg::Issued]) -> String {
    errors
        .iter()
        .map(|e| e.id.clone())
        .collect::<Vec<_>>()
        .join(",")
}

/// The `console` command.
pub fn console(rest: &str) -> Vec<String> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let (Some(id), Some(mode), Some(ms)) = (
        words.first().and_then(|w| w.parse::<u32>().ok()),
        words.get(1).copied(),
        words.get(2).and_then(|w| w.parse::<u64>().ok()),
    ) else {
        return vec!["error console <id> <view|take|second> <ms> [hex]".into()];
    };
    let inputs: Vec<ConsoleIn> = match words.get(3) {
        None => Vec::new(),
        Some(h) => match hex::decode(h).ok().and_then(|b| ipc::decode(&b).ok()) {
            Some(inputs) => inputs,
            None => return vec!["error the messages do not decode".into()],
        },
    };
    let attached = match client().console(id, mode == "take") {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => return vec![format!("error refused {}", refused(&e))],
        Err(e) => return vec![format!("error {e}")],
    };
    let (serial, mut session) = attached;
    let mut lines = vec![format!("fact serial={serial}")];
    let seen = Arc::new(Mutex::new(Seen::default()));
    let Ok(watching) = session_clone(&mut session) else {
        return vec!["error the session".into()];
    };
    let reader = {
        let seen = seen.clone();
        thread::spawn(move || {
            let mut watching = watching;
            loop {
                match watching.next(Duration::from_secs(600)) {
                    Ok(ConsoleOut::Frame { frame }) => {
                        let mut s = lock(&seen);
                        let _ = s.grid.apply(&FromSystem::Frame(frame));
                        s.frames += 1;
                        s.came.push(Instant::now());
                    }
                    Ok(ConsoleOut::Update { update }) => {
                        let mut s = lock(&seen);
                        let _ = s.grid.apply(&FromSystem::Update(update));
                        s.updates += 1;
                        s.came.push(Instant::now());
                    }
                    Ok(ConsoleOut::Serial { bytes }) => lock(&seen).serial.extend(bytes),
                    Ok(ConsoleOut::Ended { why }) => {
                        lock(&seen).ended = why;
                        return;
                    }
                    Err(_) => return,
                }
            }
        })
    };
    let mut sent = Vec::new();
    for input in &inputs {
        sent.push(Instant::now());
        if session.send(input).is_err() {
            break;
        }
        thread::sleep(APART);
    }
    if mode == "second" {
        thread::sleep(Duration::from_millis(500));
        lines.extend(second(id));
    }
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until && !reader.is_finished() {
        thread::sleep(Duration::from_millis(50));
    }
    session.close();
    let _ = reader.join();
    let s = lock(&seen);
    lines.push(format!("fact frames={}", s.frames));
    lines.push(format!("fact updates={}", s.updates));
    lines.push(format!("fact ended={}", s.ended));
    // How long after each input the screen next changed.
    let after: Vec<String> = sent
        .iter()
        .map(|t| {
            s.came.iter().find(|c| *c >= t).map_or_else(
                || "none".into(),
                |c| c.duration_since(*t).as_millis().to_string(),
            )
        })
        .collect();
    lines.push(format!("fact latency_ms={}", after.join(",")));
    if serial {
        for line in String::from_utf8_lossy(&s.serial).lines() {
            lines.push(format!("line {line}"));
        }
    } else {
        for r in 0..usize::from(s.grid.rows) {
            lines.push(format!("line {}", s.grid.row_text(r)));
        }
    }
    lines.push("ok".into());
    lines
}

/// While a viewer has `id`'s console: a second is tried, then one that
/// takes it over. What each got.
fn second(id: u32) -> Vec<String> {
    let mut lines = Vec::new();
    match client().console(id, false) {
        Ok(Err(e)) => lines.push(format!("fact second={}", refused(&e))),
        Ok(Ok((_, other))) => {
            lines.push("fact second=attached".into());
            other.close();
        }
        Err(e) => lines.push(format!("fact second=error {e}")),
    }
    match client().console(id, true) {
        Ok(Ok((_, mut other))) => {
            let first = other.next(Duration::from_secs(10));
            lines.push(format!(
                "fact takeover={}",
                if matches!(first, Ok(ConsoleOut::Frame { .. })) {
                    "frame"
                } else {
                    "attached"
                }
            ));
            other.close();
        }
        Ok(Err(e)) => lines.push(format!("fact takeover={}", refused(&e))),
        Err(e) => lines.push(format!("fact takeover=error {e}")),
    }
    lines
}

/// A second handle on the session's connection, for the reader.
fn session_clone(
    session: &mut partd::proto::ConsoleSession,
) -> io::Result<partd::proto::ConsoleSession> {
    session.try_clone()
}

struct Summed {
    bytes: u64,
    sum: u64,
}

impl Write for Summed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes += buf.len() as u64;
        self.sum += buf.iter().map(|b| u64::from(*b)).sum::<u64>();
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The `dumpread` command.
pub fn dump_read(rest: &str) -> Vec<String> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let (Some(id), Some(name)) = (
        words.first().and_then(|w| w.parse::<u32>().ok()),
        words.get(1),
    ) else {
        return vec!["error dumpread <id> <name>".into()];
    };
    let mut summed = Summed { bytes: 0, sum: 0 };
    match client().read_dump_into(id, name, &mut summed) {
        Ok(Ok(size)) if size == summed.bytes => vec![format!("ok {} {}", summed.bytes, summed.sum)],
        Ok(Ok(size)) => vec![format!("error {size} bytes said, {} read", summed.bytes)],
        Ok(Err(e)) => vec![format!("error refused {}", refused(&e))],
        Err(e) => vec![format!("error {e}")],
    }
}
