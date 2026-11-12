// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One running guest: its serial output with expect-style matching,
//! the test hooks' channel and the monitor.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::error::HarnessError;
use crate::qemu::channel::Channel;
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
    hook: Option<PathBuf>,
    monitor: Option<PathBuf>,
    channel: Option<Channel>,
}

impl Session {
    /// Launch QEMU.
    pub fn spawn(launch: &Launch) -> Result<Self, HarnessError> {
        // QEMU makes the sockets; stale ones would be connected to.
        for socket in [&launch.hook, &launch.monitor].into_iter().flatten() {
            let _ = std::fs::remove_file(socket);
        }
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
            hook: launch.hook.clone(),
            monitor: launch.monitor.clone(),
            channel: None,
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

    /// Wait until the whole serial log so far satisfies `found`, by
    /// `deadline`: for what may have been said before the last match
    /// as well as after it. Nothing is consumed.
    pub fn wait_for(
        &mut self,
        what: &str,
        deadline: Instant,
        found: impl Fn(&str) -> bool,
    ) -> Result<(), String> {
        loop {
            if found(&self.log) {
                return Ok(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(chunk) => {
                    self.buf.push_str(&chunk);
                    self.log.push_str(&chunk);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{what} did not appear in time"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("QEMU ended before {what} appeared"));
                }
            }
        }
    }

    /// Wait until `needle` is anywhere in the serial log.
    pub fn seen(&mut self, needle: &str, deadline: Instant) -> Result<(), String> {
        self.wait_for(&format!("`{needle}`"), deadline, |log| log.contains(needle))
    }

    /// Take what the serial port has said without waiting for more.
    pub fn drain(&mut self) {
        while let Ok(chunk) = self.rx.try_recv() {
            self.buf.push_str(&chunk);
            self.log.push_str(&chunk);
        }
    }

    /// Whether the serial log so far has `needle`, matched or not.
    pub fn said(&mut self, needle: &str) -> bool {
        self.drain();
        self.log.contains(needle)
    }

    /// The test hooks' channel, connected and answering, by `deadline`.
    pub fn channel(&mut self, deadline: Instant) -> Result<&mut Channel, String> {
        if self.channel.is_none() {
            let socket = self
                .hook
                .as_ref()
                .ok_or("this guest was launched without the channel")?;
            let mut channel = Channel::connect(socket, deadline)?;
            channel.wait_ready(deadline)?;
            self.channel = Some(channel);
        }
        self.channel.as_mut().ok_or_else(|| "no channel".into())
    }

    /// Give QEMU's monitor one command.
    pub fn monitor(&mut self, command: &str) -> Result<(), String> {
        let socket = self
            .monitor
            .as_ref()
            .ok_or("this guest was launched without a monitor")?;
        let mut stream = UnixStream::connect(socket).map_err(|e| format!("the monitor: {e}"))?;
        stream
            .write_all(format!("{command}\n").as_bytes())
            .map_err(|e| format!("the monitor: {e}"))?;
        // The monitor reads the command before the socket closes.
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    /// Wait for QEMU to end by itself, by `deadline`: the guest powered
    /// off. How long it took from now.
    pub fn wait_exit(&mut self, deadline: Instant) -> Result<Duration, String> {
        let from = Instant::now();
        loop {
            self.drain();
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.drain();
                    return Ok(from.elapsed());
                }
                Ok(None) if Instant::now() >= deadline => {
                    return Err("QEMU did not end in time: the guest did not power off".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(format!("waiting for QEMU: {e}")),
            }
        }
    }

    /// Kill the guest, as a power loss does, and take what it still
    /// had to say.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.channel = None;
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
