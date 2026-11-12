// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Signals without handlers: they are blocked and read from a
//! descriptor, so a program takes them in its own loop and nothing
//! runs asynchronously. PID 1 takes `SIGCHLD` and `SIGTERM` this way; a
//! service takes `SIGTERM` to end cleanly.

#![allow(unsafe_code)]

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsFd, BorrowedFd, FromRawFd};

/// A child ended.
pub const CHLD: i32 = libc::SIGCHLD;
/// End, in order.
pub const TERM: i32 = libc::SIGTERM;
/// The keyboard's interrupt.
pub const INT: i32 = libc::SIGINT;

/// Bytes of a `signalfd_siginfo`.
const INFO: usize = 128;

/// Signals taken through a descriptor.
#[derive(Debug)]
pub struct Signals {
    fd: File,
}

impl Signals {
    /// Block `signals` and take them from a descriptor from now on.
    /// Children do not inherit the block: `std` starts them with an
    /// empty mask.
    pub fn take(signals: &[i32]) -> io::Result<Self> {
        // SAFETY: a zeroed sigset_t is a valid argument for sigemptyset,
        // which initializes it; sigaddset and sigprocmask are given
        // that set; signalfd returns a new descriptor or -1.
        let fd = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut set);
            for signal in signals {
                libc::sigaddset(&raw mut set, *signal);
            }
            libc::sigprocmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
            libc::signalfd(-1, &raw const set, libc::SFD_NONBLOCK | libc::SFD_CLOEXEC)
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a descriptor nothing else owns.
        Ok(Self {
            fd: unsafe { File::from_raw_fd(fd) },
        })
    }

    /// The descriptor to poll for reading.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }

    /// The signals that have arrived; empty when none has.
    pub fn read(&mut self) -> Vec<i32> {
        let mut signals = Vec::new();
        let mut info = [0u8; INFO];
        while let Ok(INFO) = self.fd.read(&mut info) {
            // `ssi_signo` is the first field.
            let signo = u32::from_ne_bytes([info[0], info[1], info[2], info[3]]);
            signals.extend(i32::try_from(signo));
        }
        signals
    }
}
