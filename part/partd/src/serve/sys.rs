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
/// process's session reaches it. `files` are the child's descriptors 3,
/// 4 and on, in order: what the partition manager holds for its VMM.
pub fn spawn_with(command: &mut Command, procs: File, files: Vec<File>) -> io::Result<Child> {
    // Each is first moved above where any of them go, so that putting
    // one in its place cannot close another.
    let mut high = Vec::with_capacity(files.len());
    for f in &files {
        high.push(rustix::io::fcntl_dupfd_cloexec(f, 64).map_err(io::Error::from)?);
    }
    drop(files);
    let raw: Vec<i32> = high.iter().map(std::os::fd::AsRawFd::as_raw_fd).collect();
    // SAFETY: the closure runs in the child between `fork` and `exec`,
    // where only async-signal-safe calls are allowed. It makes two
    // system calls directly, `setsid` and a `write` of one byte to a
    // descriptor opened before the fork, and allocates nothing.
    unsafe {
        command.pre_exec(move || {
            for (i, fd) in raw.iter().enumerate() {
                // `dup2` leaves the new descriptor open across `exec`.
                #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
                if libc::dup2(*fd, 3 + i as i32) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
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
    let child = command.spawn();
    drop(high);
    child
}
