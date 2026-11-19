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
    // Each, and the group's file, is first moved above where any of
    // them go, so that putting one in its place cannot close another.
    let above = above(files.len());
    let mut high = Vec::with_capacity(files.len());
    for f in &files {
        high.push(rustix::io::fcntl_dupfd_cloexec(f, above).map_err(io::Error::from)?);
    }
    drop(files);
    let moved = rustix::io::fcntl_dupfd_cloexec(&procs, above).map_err(io::Error::from)?;
    drop(procs);
    let procs = moved;
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

/// The least descriptor above every place `count` files go (3 and on),
/// and above the few a process has open below 64.
fn above(count: usize) -> i32 {
    i32::try_from(count.saturating_add(3)).map_or(i32::MAX, |top| top.max(64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn more_files_than_fit_below_64_each_land_in_their_place() {
        let dir = std::env::temp_dir().join(format!("partd-spawn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let count = 80;
        let files: Vec<File> = (0..count)
            .map(|i| File::create(dir.join(format!("f{i}"))).unwrap())
            .collect();
        let procs = File::create(dir.join("procs")).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(format!(
                "i=3; while [ $i -lt {} ]; do readlink /proc/self/fd/$i; i=$((i+1)); done",
                count + 3
            ))
            .stdout(Stdio::piped());
        let out = spawn_with(&mut command, procs, files)
            .unwrap()
            .wait_with_output()
            .unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let landed: Vec<&str> = text.lines().collect();
        assert_eq!(landed.len(), count);
        for (i, path) in landed.iter().enumerate() {
            assert!(path.ends_with(&format!("/f{i}")), "{i}: {path}");
        }
        assert_eq!(std::fs::read_to_string(dir.join("procs")).unwrap(), "0");
        let _ = std::fs::remove_dir_all(dir);
    }
}
