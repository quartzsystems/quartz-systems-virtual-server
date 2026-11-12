// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `SIGCHLD` without a handler: the signal is blocked and waited for,
//! so reaping happens in the main loop and nothing runs asynchronously.

use std::time::Duration;

/// The set holding `SIGCHLD`.
fn chld() -> libc::sigset_t {
    // SAFETY: a zeroed sigset_t is a valid argument for sigemptyset,
    // which initializes it; sigaddset is given that set and a valid signal.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut set);
        libc::sigaddset(&raw mut set, libc::SIGCHLD);
        set
    }
}

/// Block `SIGCHLD` so [`wait`] can take it.
pub fn block() {
    let set = chld();
    // SAFETY: `set` is initialized; the old mask is not asked for.
    unsafe {
        libc::sigprocmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
    }
}

/// Wait up to `timeout` for a `SIGCHLD`. True when one arrived.
pub fn wait(timeout: Duration) -> bool {
    let set = chld();
    // The field's type by its width: libc's name for it is in flux on musl.
    let secs = i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX);
    let ts = libc::timespec {
        tv_sec: secs,
        tv_nsec: libc::c_long::from(timeout.subsec_nanos()),
    };
    // SAFETY: `set` and `ts` are initialized and outlive the call; no
    // siginfo is asked for.
    let signal = unsafe { libc::sigtimedwait(&raw const set, std::ptr::null_mut(), &raw const ts) };
    signal == libc::SIGCHLD
}

/// Reap every child that has ended. Returns how many.
pub fn reap() -> usize {
    let mut reaped = 0;
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid place for the exit status.
        let pid = unsafe { libc::waitpid(-1, &raw mut status, libc::WNOHANG) };
        if pid <= 0 {
            return reaped;
        }
        reaped += 1;
    }
}
