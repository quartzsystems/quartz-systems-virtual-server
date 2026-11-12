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
use std::os::unix::process::CommandExt;
use std::process::Command;

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
    /// A child inherits the block, so one that is to be ended by a
    /// signal is started through [`unblocked`].
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

/// Have `command`'s child start with no signal blocked. A child
/// inherits its parent's mask, and a parent that takes `SIGTERM` from
/// a descriptor would otherwise start children `SIGTERM` cannot end.
pub fn unblocked(command: &mut Command) -> &mut Command {
    // SAFETY: the closure runs in the child between fork and exec and
    // calls only sigemptyset and sigprocmask, which are
    // async-signal-safe; the set is a local it initializes.
    unsafe {
        command.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut set);
            if libc::sigprocmask(libc::SIG_SETMASK, &raw const set, std::ptr::null_mut()) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::time::Duration;

    use super::*;

    /// Start `sleep`, which leaves its signal mask alone, send it
    /// `SIGTERM` and say whether that ended it.
    fn ended_by_sigterm(command: &mut Command) -> bool {
        let mut child = command.arg("5").spawn().unwrap();
        let pid = libc::pid_t::try_from(child.id()).unwrap();
        // SAFETY: kill takes a process and a signal; the process is
        // this test's own child.
        unsafe { libc::kill(pid, TERM) };
        for _ in 0..40 {
            if let Some(status) = child.try_wait().unwrap() {
                return status.signal() == Some(TERM);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        false
    }

    #[test]
    fn a_child_started_unblocked_can_be_ended_by_sigterm() {
        // This thread takes SIGTERM from a descriptor, as PID 1 does.
        let mut signals = Signals::take(&[TERM]).unwrap();
        assert!(signals.read().is_empty());
        // The block is inherited: SIGTERM stays pending in the child.
        assert!(!ended_by_sigterm(&mut Command::new("/bin/sleep")));
        // Started unblocked, the child ends.
        assert!(ended_by_sigterm(unblocked(&mut Command::new("/bin/sleep"))));
    }
}
