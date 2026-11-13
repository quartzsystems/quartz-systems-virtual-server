// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A mirrored load source whose units each ran without the other. The
//! storage service assembles nothing and says so with a file; PID 1
//! then runs the Service Tools console in its one-panel form, which
//! asks the operator which unit to keep and tells the storage service.
//! It has the display and the serial port meanwhile. Nobody signs on:
//! the service tools users are on the data file system this is about.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::host::{Host, table};

/// Where what the console says besides its screens is kept.
const LOG: &str = "/run/keep-unit.log";
/// How soon a console that ended without a choice is run again.
const AGAIN: Duration = Duration::from_secs(3);

fn log() -> Stdio {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG)
        .map_or_else(|_| Stdio::null(), Stdio::from)
}

/// The console that asks, while it is wanted.
pub struct Chooser {
    marker: PathBuf,
    child: Option<Child>,
    /// The serial port is the console's.
    lent: bool,
    /// When a console may next be started.
    next: Instant,
}

impl Chooser {
    /// A chooser that looks for the storage service's `marker`.
    pub fn new(marker: PathBuf) -> Self {
        Self {
            marker,
            child: None,
            lent: false,
            next: Instant::now(),
        }
    }

    /// Whether the storage service waits for the operator. While it
    /// does, the console runs; when it does no more, the serial port
    /// is PID 1's again.
    pub fn wanted(&mut self, host: &mut Host) -> bool {
        if !self.marker.exists() {
            self.end(host);
            return false;
        }
        // PID 1 reaps every child in its loop: a console that is no
        // longer there shows as an error here, or as its status.
        let running = self
            .child
            .as_mut()
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        if !running && Instant::now() >= self.next {
            if self.child.take().is_some() {
                host.lend_serial(false);
                host.line("The console ended before a unit was chosen; it starts again");
            } else {
                host.line("The units of the load source ran apart: the console asks which to keep");
            }
            self.next = Instant::now() + AGAIN;
            let mut command = Command::new(table::TOOLS.binary);
            command
                .arg(release::KEEP_UNIT)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(log())
                .stderr(log());
            // PID 1 blocks the signals it reads from a descriptor.
            match supervise::signal::unblocked(&mut command).spawn() {
                Ok(child) => {
                    self.child = Some(child);
                    self.lent = true;
                    host.lend_serial(true);
                }
                Err(e) => host.line(&format!("{}: {e}", table::TOOLS.binary)),
            }
        }
        true
    }

    /// The choice is made, or the wait is over: the console ends and
    /// the serial port is PID 1's again.
    pub fn end(&mut self, host: &mut Host) {
        if let Some(mut child) = self.child.take() {
            // It ends by itself once it has been answered; one that
            // has not is ended.
            let give_up = Instant::now() + Duration::from_secs(2);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < give_up {
                host.pump();
            }
            // Only one that is still there: the loop has reaped any
            // other, and its number may be another process's by now.
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
        }
        if self.lent {
            self.lent = false;
            host.lend_serial(false);
        }
    }
}
