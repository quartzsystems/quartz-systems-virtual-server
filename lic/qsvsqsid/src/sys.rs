// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the server does as root before it becomes its own user: its
//! directory, its local socket, and the drop itself.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;

fn last_error(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The server's own directory: the consoles and the event log, its
/// user's and nobody else's.
pub fn own_directory(dir: &Path, user: u32) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::os::unix::fs::chown(dir, Some(user), Some(user))?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    for entry in std::fs::read_dir(dir)? {
        std::os::unix::fs::chown(entry?.path(), Some(user), Some(user))?;
    }
    Ok(())
}

/// The local socket, root's alone: the server keeps it open after the
/// drop, and Service Tools, which is root, connects.
pub fn local_socket(path: &Path) -> io::Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Become user and group `id` for good, with one supplementary group
/// (its own, which the services' sockets are shared with), no
/// capability in any set, and no way to gain one through an exec.
/// Called while the process has one thread.
pub fn drop_privileges(id: u32) -> io::Result<()> {
    let groups = [id];
    // SAFETY: each call takes plain integers, or a pointer to a local
    // array with its length, and changes only this process's
    // credentials.
    unsafe {
        for capability in 0..64 {
            if libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::EINVAL) {
                    break;
                }
                return Err(error);
            }
        }
        last_error(libc::setgroups(groups.len(), groups.as_ptr()))?;
        last_error(libc::setresgid(id, id, id))?;
        last_error(libc::setresuid(id, id, id))?;
        last_error(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
        if libc::setresuid(0, 0, 0) == 0 || libc::getuid() != id || libc::geteuid() != id {
            return Err(io::Error::other("privileges were not dropped"));
        }
    }
    Ok(())
}

/// Block SIGTERM in this thread and every thread started after it, and
/// take it on a thread of its own: `then` is called once it comes.
pub fn on_term(then: impl FnOnce() + Send + 'static) -> io::Result<()> {
    // SAFETY: the set is a zeroed local, filled by `sigemptyset` and
    // `sigaddset`, and no old set is asked for.
    let set = unsafe {
        let mut set = std::mem::zeroed::<libc::sigset_t>();
        last_error(libc::sigemptyset(&raw mut set))?;
        last_error(libc::sigaddset(&raw mut set, libc::SIGTERM))?;
        let r = libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
        if r != 0 {
            return Err(io::Error::from_raw_os_error(r));
        }
        set
    };
    std::thread::Builder::new()
        .name("term".into())
        .spawn(move || {
            let mut signal: libc::c_int = 0;
            // SAFETY: both pointers are to initialized values of the types
            // `sigwait` reads and writes.
            if unsafe { libc::sigwait(&raw const set, &raw mut signal) } == 0 {
                then();
            }
        })?;
    Ok(())
}
