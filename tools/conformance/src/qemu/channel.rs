// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The harness's end of the test hooks' channel: the guest's second
//! serial port, which QEMU gives the host as a Unix socket. A test
//! image answers on it; a release image has nothing there.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use qsvshook::proto::{END, command, parse_status, parse_storage, word};
use supervise::control::HostStatus;

/// How long an ordinary command may take.
const ANSWER: Duration = Duration::from_secs(10);

/// The channel.
pub struct Channel {
    stream: UnixStream,
    /// Received and not yet taken as lines.
    pending: Vec<u8>,
    pings: u32,
}

impl Channel {
    /// Connect to the socket QEMU listens on, by `deadline`.
    pub fn connect(socket: &Path, deadline: Instant) -> Result<Self, String> {
        loop {
            match UnixStream::connect(socket) {
                Ok(stream) => {
                    return Ok(Self {
                        stream,
                        pending: Vec::new(),
                        pings: 0,
                    });
                }
                Err(e) if Instant::now() >= deadline => {
                    return Err(format!("{}: {e}", socket.display()));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// The next line, by `deadline`.
    fn line(&mut self, deadline: Instant) -> Result<String, String> {
        loop {
            if let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                let text = String::from_utf8_lossy(&line);
                return Ok(text.trim_end_matches(['\r', '\n']).to_string());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the channel did not answer in time".into());
            }
            self.stream
                .set_read_timeout(Some(left))
                .map_err(|e| e.to_string())?;
            let mut buf = [0u8; 4096];
            match self.stream.read(&mut buf) {
                Ok(0) => return Err("the channel closed: QEMU ended".into()),
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(format!("the channel: {e}")),
            }
        }
    }

    /// The lines of the next answer.
    fn answer(&mut self, deadline: Instant) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        loop {
            let line = self.line(deadline)?;
            if line == END {
                return Ok(lines);
            }
            lines.push(line);
        }
    }

    fn send(&mut self, command: &str) -> Result<(), String> {
        self.stream
            .write_all(format!("{command}\n").as_bytes())
            .map_err(|e| format!("the channel: {e}"))
    }

    /// Send a command without waiting for its answer: the shutdown
    /// ends the guest's end of the channel before it can give one.
    pub fn tell(&mut self, command: &str) -> Result<(), String> {
        self.send(command)
    }

    /// Wait until the guest's end answers, by `deadline`. Each ping
    /// carries a number, so an answer to an earlier one, or to what
    /// the port made of bytes sent before it was open, is passed over.
    pub fn wait_ready(&mut self, deadline: Instant) -> Result<(), String> {
        while Instant::now() < deadline {
            self.pings += 1;
            let wanted = format!("pong {}", self.pings);
            self.send(&format!("{} {}", command::PING, self.pings))?;
            let by = deadline.min(Instant::now() + Duration::from_secs(1));
            while let Ok(lines) = self.answer(by) {
                if lines == [wanted.as_str()] {
                    return Ok(());
                }
            }
        }
        Err("the test hooks' channel did not answer: is this a test image?".into())
    }

    /// Send one command and take its answer. An `error` line is an error.
    pub fn ask(&mut self, command: &str) -> Result<Vec<String>, String> {
        self.send(command)?;
        let lines = self.answer(Instant::now() + ANSWER)?;
        match lines.first().and_then(|l| l.split_once(' ')) {
            Some((word::ERROR, why)) => Err(format!("`{command}`: {why}")),
            _ => Ok(lines),
        }
    }

    /// The control socket's status.
    pub fn status(&mut self) -> Result<HostStatus, String> {
        parse_status(&self.ask(command::STATUS)?)
    }

    /// The storage service's status.
    pub fn storage(&mut self) -> Result<stgd::Status, String> {
        parse_storage(&self.ask(command::STORAGE)?)
    }

    /// Poll the storage service's status until `done`, by `deadline`.
    pub fn storage_until(
        &mut self,
        what: &str,
        deadline: Instant,
        done: impl Fn(&stgd::Status) -> bool,
    ) -> Result<stgd::Status, String> {
        loop {
            let status = self.storage();
            if let Ok(status) = &status
                && done(status)
            {
                return Ok(status.clone());
            }
            if Instant::now() >= deadline {
                return Err(format!("{what}: not in time; storage is {status:?}"));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// A file's length and the sum of its bytes.
    pub fn sum(&mut self, path: &str) -> Result<(u64, u64), String> {
        let lines = self.ask(&format!("{} {path}", command::SUM))?;
        let parts: Vec<&str> = lines.first().map_or(Vec::new(), |l| l.split(' ').collect());
        match parts.as_slice() {
            [word::OK, len, sum] => Ok((
                len.parse()
                    .map_err(|_| format!("`sum {path}`: {lines:?}"))?,
                sum.parse()
                    .map_err(|_| format!("`sum {path}`: {lines:?}"))?,
            )),
            _ => Err(format!("`sum {path}` answered {lines:?}")),
        }
    }

    /// A file's bytes.
    pub fn cat(&mut self, path: &str) -> Result<Vec<u8>, String> {
        let lines = self.ask(&format!("{} {path}", command::CAT))?;
        let hex = match lines.as_slice() {
            [line] => line.strip_prefix(word::HEX).map(str::trim),
            _ => None,
        };
        hex.and_then(|h| hex::decode(h).ok())
            .ok_or_else(|| format!("`cat {path}` answered {lines:?}"))
    }

    /// A text file.
    pub fn text(&mut self, path: &str) -> Result<String, String> {
        String::from_utf8(self.cat(path)?).map_err(|e| format!("{path}: {e}"))
    }

    /// The names in a directory.
    pub fn ls(&mut self, dir: &str) -> Result<Vec<String>, String> {
        let lines = self.ask(&format!("{} {dir}", command::LS))?;
        Ok(lines
            .iter()
            .filter_map(|l| l.strip_prefix(word::NAME))
            .map(|n| n.trim().to_string())
            .collect())
    }

    /// The kernel log's lines that contain `text`.
    pub fn kmsg(&mut self, text: &str) -> Result<Vec<String>, String> {
        let lines = self.ask(&format!("{} {text}", command::KMSG))?;
        Ok(lines
            .iter()
            .filter_map(|l| l.strip_prefix(word::LINE))
            .map(|n| n.trim().to_string())
            .collect())
    }

    /// Poll the status until `done` says so, by `deadline`.
    pub fn until(
        &mut self,
        what: &str,
        deadline: Instant,
        done: impl Fn(&HostStatus) -> bool,
    ) -> Result<HostStatus, String> {
        loop {
            let status = self.status()?;
            if done(&status) {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(format!("{what}: not in time; the status is {status:?}"));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}
