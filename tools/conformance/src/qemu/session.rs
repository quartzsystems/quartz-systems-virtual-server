// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One running guest: its serial output with expect-style matching,
//! the test hooks' channel and the monitor.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::error::HarnessError;
use crate::qemu::channel::Channel;
use crate::qemu::term::Grid;
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
    /// The guest's serial input, when the launch asked for it.
    keys: Option<ChildStdin>,
    /// The serial port as a terminal shows it.
    grid: Grid,
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
            .stdin(if launch.keys {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| HarnessError::Spawn {
                qemu: launch.qemu.clone(),
                source,
            })?;
        let keys = child.stdin.take();
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
            keys,
            grid: Grid::default(),
        })
    }

    /// Take a chunk of what the guest said.
    fn take(&mut self, chunk: &str) {
        self.buf.push_str(chunk);
        self.log.push_str(chunk);
        self.grid.feed(chunk);
    }

    /// Type `text` at the guest's serial port.
    pub fn type_keys(&mut self, text: &str) -> Result<(), String> {
        let keys = self
            .keys
            .as_mut()
            .ok_or("this guest was launched without serial input")?;
        keys.write_all(text.as_bytes())
            .and_then(|()| keys.flush())
            .map_err(|e| format!("typing at the serial port: {e}"))
    }

    /// The serial port as a terminal shows it now.
    pub fn grid(&mut self) -> &Grid {
        self.drain();
        &self.grid
    }

    /// Wait until the terminal's grid satisfies `found`, by `deadline`:
    /// the grid's text then. The grid must also have stopped changing
    /// for a moment, so a screen half drawn is not taken.
    pub fn screen(
        &mut self,
        what: &str,
        deadline: Instant,
        found: impl Fn(&Grid) -> bool,
    ) -> Result<String, String> {
        let settle = Duration::from_millis(150);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let wait = if found(&self.grid) { settle } else { left };
            match self.rx.recv_timeout(wait) {
                Ok(chunk) => self.take(&chunk),
                Err(_) if found(&self.grid) => return Ok(self.grid.text()),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "{what} did not appear in time; the terminal shows:\n{}",
                        self.grid.text()
                    ));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("QEMU ended before {what} appeared"));
                }
            }
        }
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
                Ok(chunk) => self.take(&chunk),
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

    /// Wait until `needle` appears on a line that starts with `prefix`,
    /// also after another writer's text in the middle of that line, by
    /// `deadline`. Everything up to and including it is consumed.
    pub fn expect_on_line(
        &mut self,
        prefix: &str,
        needle: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut from = 0;
        loop {
            while let Some(found) = self.buf.get(from..).and_then(|b| b.find(needle)) {
                let at = from + found;
                let abs = self.log.len() - self.buf.len() + at;
                let start = self.log[..abs].rfind('\n').map_or(0, |i| i + 1);
                if self.log[start..].starts_with(prefix) {
                    self.buf.drain(..at + needle.len());
                    return Ok(());
                }
                from = at + needle.len();
            }
            // A match may straddle what arrives next.
            from = from.max(self.buf.len().saturating_sub(needle.len()));
            while !self.buf.is_char_boundary(from) {
                from -= 1;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let gone = || format!("`{prefix}{needle}`");
            if left.is_zero() {
                return Err(format!("{} did not appear in time", gone()));
            }
            match self.rx.recv_timeout(left) {
                Ok(chunk) => self.take(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{} did not appear in time", gone()));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("QEMU ended before {} appeared", gone()));
                }
            }
        }
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
                Ok(chunk) => self.take(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("{what} did not appear in time"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(format!("QEMU ended before {what} appeared"));
                }
            }
        }
    }

    /// Wait until `needle` is anywhere in the serial log, also where a
    /// kernel message was written into the middle of its line.
    pub fn seen(&mut self, needle: &str, deadline: Instant) -> Result<(), String> {
        self.wait_for(&format!("`{needle}`"), deadline, |log| {
            log.contains(needle) || without_kernel(log).contains(needle)
        })
    }

    /// The last line of the serial log that has `needle`, once one has
    /// appeared, by `deadline`. Nothing is consumed.
    pub fn line_with(&mut self, needle: &str, deadline: Instant) -> Result<String, String> {
        self.seen(needle, deadline)?;
        let at = self.log.rfind(needle).unwrap_or(0);
        let start = self.log[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = self.log[at..].find('\n').map_or(self.log.len(), |i| at + i);
        Ok(self.log[start..end].trim_end().to_string())
    }

    /// Take what the serial port has said without waiting for more.
    pub fn drain(&mut self) {
        while let Ok(chunk) = self.rx.try_recv() {
            self.take(&chunk);
        }
    }

    /// Where in the serial log so far `needle` first is.
    pub fn log_position(&mut self, needle: &str) -> Option<usize> {
        self.drain();
        self.log.find(needle)
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

    /// Type keys at the guest's keyboard, through QEMU's monitor: each
    /// of `keys` is a `sendkey` name (`a`, `shift-a`, `ret`, `f10`).
    pub fn send_keys(&mut self, keys: &[String]) -> Result<(), String> {
        let socket = self
            .monitor
            .as_ref()
            .ok_or("this guest was launched without a monitor")?;
        let mut stream = UnixStream::connect(socket).map_err(|e| format!("the monitor: {e}"))?;
        for key in keys {
            stream
                .write_all(format!("sendkey {key} 20\n").as_bytes())
                .map_err(|e| format!("the monitor: {e}"))?;
            // A key is down for 20 ms; the next follows its release.
            std::thread::sleep(Duration::from_millis(60));
        }
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
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

/// `log` without the kernel's messages: each `[ seconds.micros] ...`
/// up to its line's end, wherever on a line it began.
fn without_kernel(log: &str) -> String {
    let bytes = log.as_bytes();
    let mut out = String::with_capacity(log.len());
    let mut from = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(len) = stamp(&bytes[i..])
        {
            out.push_str(&log[from..i]);
            let end = log[i + len..]
                .find(NL)
                .map_or(log.len(), |n| i + len + n + 1);
            i = end;
            from = end;
        } else {
            i += 1;
        }
    }
    out.push_str(&log[from..]);
    out
}

/// A line's end.
const NL: char = '\x0a';

/// The length of a kernel message's time stamp, `[` spaces, digits, `.`,
/// six digits, `] `, at the start of `b`.
fn stamp(b: &[u8]) -> Option<usize> {
    let mut i = 1;
    while b.get(i) == Some(&b' ') {
        i += 1;
    }
    let whole = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == whole || b.get(i) != Some(&b'.') {
        return None;
    }
    i += 1;
    let frac = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    (i - frac == 6 && b.get(i) == Some(&b']') && b.get(i + 1) == Some(&b' ')).then_some(i + 2)
}

#[cfg(test)]
mod kernel_tests {
    use super::without_kernel;

    #[test]
    fn a_kernel_message_inside_a_line_is_taken_out() {
        let log = "REF A2007020 Pool unit failed: P[    1.950008] I/O error, dev vdb\x0d\x0aOOL1, unit 1\x0a";
        assert_eq!(
            without_kernel(log),
            "REF A2007020 Pool unit failed: POOL1, unit 1\x0a"
        );
    }

    #[test]
    fn other_brackets_stay() {
        let log = "unit(s) [0] pulled; [1.5] and [ 12.123456]x stay\x0a";
        assert_eq!(without_kernel(log), log);
    }
}
