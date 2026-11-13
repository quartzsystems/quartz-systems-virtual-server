// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The one call that needs `unsafe`: starting a process that is in
//! its control group, and in a session of its own, before it runs.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

/// Start `command` as a member of the control group whose
/// `cgroup.procs` is `procs`, in a session of its own: it is in the
/// group before its first instruction, and no signal sent to this
/// process's session reaches it.
pub fn spawn_in_group(command: &mut Command, procs: File) -> io::Result<Child> {
    // SAFETY: the closure runs in the child between `fork` and `exec`,
    // where only async-signal-safe calls are allowed. It makes two
    // system calls directly, `setsid` and a `write` of one byte to a
    // descriptor opened before the fork, and allocates nothing.
    unsafe {
        command.pre_exec(move || {
            rustix::process::setsid().map_err(io::Error::from)?;
            // "0" is the process that writes.
            let written = rustix::io::write(procs.as_fd(), b"0").map_err(io::Error::from)?;
            if written == 1 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(libc::EIO))
            }
        });
    }
    command.spawn()
}
