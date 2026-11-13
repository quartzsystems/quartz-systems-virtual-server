// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Watching a VMM: its report is followed a line at a time, and its
//! end is taken. A VMM this manager started is waited for as its
//! child; one found running is another process's child by now, so its
//! control group is watched for the moment it empties, and how it
//! ended is the last line of its report.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use std::os::unix::process::ExitStatusExt;

use super::machine::{Machine, Watch};
use crate::manager::{Ended, MAX_LINE, Manager};

/// The manager, shared by every thread of the service.
pub type Shared = Arc<Mutex<Manager<Machine>>>;

/// How often a report and a control group are looked at.
const PAUSE: Duration = Duration::from_millis(25);
/// The most bytes of a report read at once.
const CHUNK: usize = 16 * 1024;

/// The word that starts the line a VMM ends its report with.
pub use qsvmm::END_WORD;

/// What a report's last line says of how its VMM ended.
pub fn end_of(report: &str) -> Ended {
    report
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(END_WORD)?.trim().parse::<i32>().ok())
        .map_or(Ended::Unknown, Ended::Exit)
}

/// Whether a control group has a process that has not ended, by the
/// group's own events: one that has ended and waits to be reaped does
/// not count.
pub fn populated(group: &Path) -> bool {
    fs::read_to_string(group.join("cgroup.events"))
        .is_ok_and(|events| events.lines().any(|line| line.trim() == "populated 1"))
}

/// The lines of the report at `path`, bounded: a few hundred.
pub fn lines_of(path: &Path) -> (Vec<String>, u64) {
    let mut text = String::new();
    let read = File::open(path)
        .and_then(|file| file.take(1 << 20).read_to_string(&mut text))
        .unwrap_or(0);
    let lines = text
        .lines()
        .map(|line| line.chars().take(MAX_LINE).collect())
        .collect();
    (lines, read as u64)
}

/// Follows a report from an offset: each whole line as it arrives.
struct Tail {
    file: Option<File>,
    partial: Vec<u8>,
}

impl Tail {
    fn open(path: &Path, from: u64) -> Self {
        let file = File::open(path).ok().and_then(|mut file| {
            file.seek(SeekFrom::Start(from)).ok()?;
            Some(file)
        });
        Self {
            file,
            partial: Vec::new(),
        }
    }

    /// The whole lines that have arrived since the last call.
    fn lines(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(file) = &mut self.file else {
            return out;
        };
        let mut chunk = vec![0u8; CHUNK];
        while let Ok(n) = file.read(&mut chunk) {
            if n == 0 {
                break;
            }
            for byte in &chunk[..n] {
                if *byte == b'\n' {
                    out.push(String::from_utf8_lossy(&self.partial).into_owned());
                    self.partial.clear();
                } else if self.partial.len() < MAX_LINE {
                    self.partial.push(*byte);
                }
            }
        }
        out
    }
}

fn lock(shared: &Shared) -> std::sync::MutexGuard<'_, Manager<Machine>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Watch one VMM to its end.
pub fn watch(shared: &Shared, watch: Watch) {
    let Watch {
        id,
        child,
        group,
        report,
        from,
    } = watch;
    let done = Arc::new(AtomicBool::new(false));
    let last = Arc::new(Mutex::new(Ended::Unknown));
    let follower = {
        let (shared, done, last) = (shared.clone(), done.clone(), last.clone());
        thread::spawn(move || {
            let mut tail = Tail::open(&report, from);
            loop {
                // Whether it has ended is asked first: what it wrote
                // before that is then all read.
                let over = done.load(Ordering::Acquire);
                for line in tail.lines() {
                    if let Some(code) = line
                        .strip_prefix(END_WORD)
                        .and_then(|rest| rest.trim().parse().ok())
                    {
                        *last.lock().unwrap_or_else(PoisonError::into_inner) = Ended::Exit(code);
                    } else {
                        lock(&shared).report(id, &line, false);
                    }
                }
                if over {
                    return;
                }
                thread::sleep(PAUSE);
            }
        })
    };
    let how = if let Some(mut child) = child {
        match child.wait().map(|status| (status.code(), status.signal())) {
            Ok((Some(code), _)) => Ended::Exit(code),
            Ok((None, Some(signal))) => Ended::Signal(signal),
            Ok((None, None)) | Err(_) => Ended::Unknown,
        }
    } else {
        while populated(&group) {
            thread::sleep(PAUSE);
        }
        Ended::Unknown
    };
    done.store(true, Ordering::Release);
    let _ = follower.join();
    // A VMM that was not this manager's child said how it ended
    // itself, or was killed before it could.
    let how = match how {
        Ended::Unknown => *last.lock().unwrap_or_else(PoisonError::into_inner),
        known => known,
    };
    lock(shared).ended(id, how);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn a_report_s_last_line_says_how_a_vmm_that_was_not_waited_for_ended() {
        assert_eq!(end_of("REF C2005100 x\nEND 0\n"), Ended::Exit(0));
        assert_eq!(end_of("END 10\nqsvmm: late line\n"), Ended::Exit(10));
        assert_eq!(end_of("REF C2005090 Partition started\n"), Ended::Unknown);
        assert_eq!(end_of("END nothing\n"), Ended::Unknown);
        assert_eq!(end_of(""), Ended::Unknown);
    }

    #[test]
    fn a_report_is_followed_a_whole_line_at_a_time_from_where_it_was_left() {
        let path = std::env::temp_dir().join(format!("partd-tail-{}", std::process::id()));
        let mut file = File::create(&path).unwrap();
        file.write_all(b"REF C2005000 first\nREF C2005010 sec")
            .unwrap();
        let (lines, read) = lines_of(&path);
        assert_eq!(lines, ["REF C2005000 first", "REF C2005010 sec"]);
        assert_eq!(read, 35);
        let mut tail = Tail::open(&path, 19);
        assert!(tail.lines().is_empty());
        file.write_all(b"ond\nthird\n").unwrap();
        assert_eq!(tail.lines(), ["REF C2005010 second", "third"]);
        // A line with no end is cut at the bound, not kept for ever.
        file.write_all(&vec![b'x'; 10 * MAX_LINE]).unwrap();
        file.write_all(b"\nlast\n").unwrap();
        let lines = tail.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), MAX_LINE);
        assert_eq!(lines[1], "last");
        std::fs::remove_file(&path).unwrap();
        assert!(Tail::open(&path, 0).lines().is_empty());
        assert!(!populated(&path));
    }
}
