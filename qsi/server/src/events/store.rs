// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The event log on disk: one host sequence, bounded as the session's
//! log is, so a server started again numbers on from where it was and
//! a console that reconnects is given what it missed.
//!
//! The log is taken up with `qsisess::EventLog::resume`: the events
//! kept, with their own numbers, and the next number after them.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use qsisess::{Event, EventLog, Limits};

/// The file, under the server's state directory.
pub const FILE: &str = "qsi-events.log";

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

pub(super) fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// One line: `seq family op body` in hexadecimal.
fn line(e: &Event) -> String {
    format!("{} {} {} {}\n", e.seq, e.family, e.op, hex(&e.body))
}

fn parse(l: &str) -> Option<Event> {
    let mut f = l.split(' ');
    let seq = f.next()?.parse().ok()?;
    let family = f.next()?.parse().ok()?;
    let op = f.next()?.parse().ok()?;
    let body = unhex(f.next().unwrap_or(""))?;
    if f.next().is_some() || seq == 0 {
        return None;
    }
    Some(Event {
        seq,
        family,
        op,
        body,
    })
}

/// The log on disk, as the session's log is bounded.
#[derive(Debug)]
pub struct Store {
    path: PathBuf,
    file: Option<File>,
    kept: VecDeque<Event>,
    bytes: usize,
    max_events: usize,
    max_bytes: usize,
    /// Lines written since the file was last made again.
    lines: usize,
}

impl Store {
    /// The store in `dir`, and the log taken up from it: the last run
    /// of events it kept numbered one after another, numbering on after
    /// the last. A line that does not read is passed over; a gap or a
    /// step back starts the run again.
    pub fn open(dir: &Path, limits: &Limits) -> (Self, EventLog) {
        let path = dir.join(FILE);
        let mut kept: VecDeque<Event> = VecDeque::new();
        if let Ok(f) = File::open(&path) {
            for l in BufReader::new(f).lines().map_while(Result::ok) {
                let Some(e) = parse(&l) else { continue };
                if kept.back().is_some_and(|last| e.seq != last.seq + 1) {
                    kept.clear();
                }
                kept.push_back(e);
            }
        }
        let mut store = Self {
            path,
            file: None,
            kept: VecDeque::new(),
            bytes: 0,
            max_events: limits.log_events.max(1),
            max_bytes: limits.log_bytes,
            lines: 0,
        };
        let next = kept.back().map_or(1, |e| e.seq.saturating_add(1));
        // Kept events the log will not take (one longer than a message
        // now is) are dropped; the numbers still go on.
        let log = EventLog::resume(limits, next, kept.iter().cloned()).unwrap_or_else(|_| {
            kept.clear();
            EventLog::resume(limits, next, []).unwrap_or_else(|_| EventLog::new(limits))
        });
        for e in kept {
            store.keep(e);
        }
        store.rewrite();
        (store, log)
    }

    fn keep(&mut self, e: Event) {
        self.bytes += e.body.len();
        self.kept.push_back(e);
        while self.kept.len() > 1
            && (self.kept.len() > self.max_events || self.bytes > self.max_bytes)
        {
            if let Some(old) = self.kept.pop_front() {
                self.bytes -= old.body.len();
            }
        }
    }

    /// Write the kept events afresh: the file stays near the bounds.
    fn rewrite(&mut self) {
        let tmp = self.path.with_extension("tmp");
        let written = File::create(&tmp).and_then(|mut f| {
            for e in &self.kept {
                f.write_all(line(e).as_bytes())?;
            }
            f.sync_all()
        });
        if written.is_ok() && std::fs::rename(&tmp, &self.path).is_ok() {
            self.lines = self.kept.len();
        }
        self.file = OpenOptions::new().append(true).open(&self.path).ok();
    }

    /// Keep an event the session's log was given, with its number.
    pub fn append(&mut self, e: &Event) {
        self.keep(e.clone());
        if let Some(f) = &mut self.file
            && f.write_all(line(e).as_bytes()).is_ok()
        {
            self.lines += 1;
        }
        if self.lines > 2 * self.max_events.max(self.kept.len()) {
            self.rewrite();
        }
    }

    /// Bring what was appended to the disk.
    pub fn sync(&mut self) {
        if let Some(f) = &self.file {
            let _ = f.sync_data();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("qsi-events-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn push(store: &mut Store, log: &mut EventLog, n: u8) {
        let seq = log.push(0x11, 1, vec![n; 3]).unwrap();
        let e = log.from(seq).next().unwrap().clone();
        store.append(&e);
    }

    #[test]
    fn a_log_taken_up_again_numbers_on_and_replays_what_was_missed() {
        let d = dir("again");
        let limits = Limits {
            log_events: 4,
            ..Limits::default()
        };
        let (mut store, mut log) = Store::open(&d, &limits);
        assert_eq!((log.first(), log.next()), (1, 1));
        for n in 0..10 {
            push(&mut store, &mut log, n);
        }
        drop(store);
        let (_, log) = Store::open(&d, &limits);
        // Events 7 to 10 were kept; the next is 11.
        assert_eq!((log.first(), log.next()), (7, 11));
        let seqs: Vec<u64> = log.from(8).map(|e| e.seq).collect();
        assert_eq!(seqs, vec![8, 9, 10]);
        assert_eq!(log.from(8).next().unwrap().body, vec![7; 3]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_damaged_file_keeps_the_last_whole_run() {
        let d = dir("damaged");
        std::fs::write(d.join(FILE), "3 17 1 aa\n4 17 1 bb\nnonsense\n5 17 1 cc\n").unwrap();
        let (_, log) = Store::open(&d, &Limits::default());
        assert_eq!((log.first(), log.next()), (3, 6));
        std::fs::write(d.join(FILE), "3 17 1 aa\n9 17 1 bb\n10 17 1 cc\n").unwrap();
        let (_, log) = Store::open(&d, &Limits::default());
        assert_eq!(
            (log.first(), log.next()),
            (9, 11),
            "a gap starts the run again"
        );
        std::fs::write(d.join(FILE), "").unwrap();
        let (_, log) = Store::open(&d, &Limits::default());
        assert_eq!((log.first(), log.next()), (1, 1));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_file_is_made_again_near_its_bounds() {
        let d = dir("bounds");
        let limits = Limits {
            log_events: 3,
            ..Limits::default()
        };
        let (mut store, mut log) = Store::open(&d, &limits);
        for n in 0..50 {
            push(&mut store, &mut log, n);
        }
        let lines = std::fs::read_to_string(d.join(FILE))
            .unwrap()
            .lines()
            .count();
        assert!(lines <= 7, "{lines} lines");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
