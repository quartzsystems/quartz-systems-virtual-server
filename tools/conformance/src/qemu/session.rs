// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One running guest: its serial output with expect-style matching.

use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::error::HarnessError;
use crate::qemu::{self, Launch};

/// A guest under test. Killed when dropped.
pub struct Session {
    child: Child,
    rx: Receiver<String>,
    /// Received and not yet matched.
    buf: String,
    /// Everything received so far.
    pub log: String,
    started: Instant,
}

impl Session {
    /// Launch QEMU.
    pub fn spawn(launch: &Launch) -> Result<Self, HarnessError> {
        let mut child = Command::new(&launch.qemu)
            .args(qemu::args(launch))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| HarnessError::Spawn {
                qemu: launch.qemu.clone(),
                source,
            })?;
        let rx = qemu::serial::stream(child.stdout.take(), child.stderr.take());
        Ok(Self {
            child,
            rx,
            buf: String::new(),
            log: String::new(),
            started: Instant::now(),
        })
    }

    /// Time since launch.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Wait until one of `needles` appears, by `deadline`. Everything up
    /// to and including it is consumed; which one it was is returned.
    pub fn expect_any(&mut self, needles: &[&str], deadline: Instant) -> Result<usize, String> {
        let wanted = || needles.join("` or `");
        loop {
            let first = needles
                .iter()
                .enumerate()
                .filter_map(|(i, n)| self.buf.find(n).map(|at| (at, i, n.len())))
                .min();
            if let Some((at, i, len)) = first {
                self.buf.drain(..at + len);
                return Ok(i);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("`{}` did not appear in time", wanted()));
            }
            match self.rx.recv_timeout(left) {
                Ok(chunk) => {
                    self.buf.push_str(&chunk);
                    self.log.push_str(&chunk);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("`{}` did not appear in time", wanted()));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("QEMU ended before `{}` appeared", wanted()));
                }
            }
        }
    }

    /// Wait until `needle` appears, by `deadline`.
    pub fn expect(&mut self, needle: &str, deadline: Instant) -> Result<(), String> {
        self.expect_any(&[needle], deadline).map(|_| ())
    }

    /// The rest of the line the last match was on.
    pub fn rest_of_line(&mut self, deadline: Instant) -> Result<String, String> {
        let before = self.log.len() - self.buf.len();
        self.expect("\n", deadline)?;
        let after = self.log.len() - self.buf.len();
        Ok(self.log[before..after].trim_end().to_string())
    }

    /// Kill the guest and take what it still had to say.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        while let Ok(chunk) = self.rx.recv_timeout(Duration::from_millis(200)) {
            self.log.push_str(&chunk);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
