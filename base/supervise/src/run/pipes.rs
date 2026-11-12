// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The pipes PID 1 holds: one per output stream of a service for the
//! life of the host, so a restart loses nothing, and a readiness pipe
//! per start.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use rustix::fs::{OFlags, fcntl_setfl};
use rustix::pipe::{PipeFlags, pipe_with};

/// The longest line passed on whole; a longer one is cut there.
const LINE_MAX: usize = 4096;

/// A pipe: the read end not blocking, both ends closed on `exec`.
pub fn pipe() -> io::Result<(File, OwnedFd)> {
    let (read, write) = pipe_with(PipeFlags::CLOEXEC)?;
    fcntl_setfl(&read, OFlags::NONBLOCK)?;
    Ok((File::from(read), write))
}

/// One byte, `None` when there is none yet. End of file is an error.
pub fn read_byte(mut pipe: &File) -> io::Result<Option<u8>> {
    let mut byte = [0u8];
    match pipe.read(&mut byte) {
        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(_) => Ok(Some(byte[0])),
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e),
    }
}

/// A service's output stream, split into lines.
pub struct Stream {
    read: File,
    write: OwnedFd,
    partial: Vec<u8>,
}

impl Stream {
    /// A new pipe.
    pub fn new() -> io::Result<Self> {
        let (read, write) = pipe()?;
        Ok(Self {
            read,
            write,
            partial: Vec::new(),
        })
    }

    /// The end PID 1 polls.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }

    /// A writing end for a child.
    pub fn writer(&self) -> io::Result<OwnedFd> {
        self.write.try_clone()
    }

    /// The whole lines that have arrived.
    pub fn lines(&mut self) -> Vec<String> {
        let mut buf = [0u8; 4096];
        let mut lines = Vec::new();
        loop {
            let n = match (&self.read).read(&mut buf) {
                Ok(n) if n > 0 => n,
                _ => break,
            };
            for &byte in &buf[..n] {
                if byte == b'\n' || self.partial.len() >= LINE_MAX {
                    let line = String::from_utf8_lossy(&self.partial);
                    lines.push(line.trim_end_matches('\r').to_string());
                    self.partial.clear();
                }
                if byte != b'\n' {
                    self.partial.push(byte);
                }
            }
        }
        lines
    }
}
