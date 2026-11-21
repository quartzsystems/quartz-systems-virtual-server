// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The calls that need `unsafe`: giving KVM the guest's memory,
//! waiting for a signal, dropping the process's privileges, and its
//! confinement: namespaces, an empty root and seccomp filters.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;

use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::VmFd;

/// Give KVM `bytes` of this process's memory at `host` as the guest's
/// memory from `guest`.
///
/// The caller keeps the mapping for as long as the VM exists.
pub fn register_memory(
    vm: &VmFd,
    slot: u32,
    guest: u64,
    bytes: u64,
    host: *mut u8,
) -> io::Result<()> {
    let region = kvm_userspace_memory_region {
        slot,
        guest_phys_addr: guest,
        memory_size: bytes,
        userspace_addr: host as u64,
        flags: 0,
    };
    // SAFETY: `host` is the start of a mapping of `bytes` bytes owned
    // by the guest memory object, which the machine holds in an `Arc`
    // beside the VM and never unmaps while the VM is open; the region
    // does not overlap another slot's, each coming from a distinct
    // mapping.
    unsafe { vm.set_user_memory_region(region) }
        .map_err(|e| io::Error::from_raw_os_error(e.errno()))
}

fn last_error<T>(result: libc::c_int, value: T) -> io::Result<T> {
    if result == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// The signal that asks for the processors' state to be reported.
pub const REPORT: i32 = libc::SIGUSR1;

fn termination_signals() -> io::Result<libc::sigset_t> {
    // SAFETY: `sigemptyset` and `sigaddset` write only the set they
    // are given, which is a local of the right type.
    unsafe {
        let mut set = std::mem::zeroed::<libc::sigset_t>();
        last_error(libc::sigemptyset(&raw mut set), ())?;
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, REPORT] {
            last_error(libc::sigaddset(&raw mut set, signal), ())?;
        }
        Ok(set)
    }
}

/// Block the signals that ask the process to end, and the one that
/// asks for a report, in this thread and every thread started after:
/// they are taken by [`wait_for_termination`] instead of acting at once.
pub fn block_termination() -> io::Result<()> {
    let set = termination_signals()?;
    // SAFETY: the set is initialized, and no old set is asked for.
    let result =
        unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut()) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

/// Wait for one of the blocked signals; which it was.
pub fn wait_for_termination() -> io::Result<i32> {
    let set = termination_signals()?;
    let mut signal: libc::c_int = 0;
    // SAFETY: both pointers are to initialized locals of the types
    // `sigwait` reads and writes.
    let result = unsafe { libc::sigwait(&raw const set, &raw mut signal) };
    if result == 0 {
        Ok(signal)
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

/// `prctl`'s core scheduling request and its two arguments used here.
const PR_SCHED_CORE: libc::c_int = 62;
const PR_SCHED_CORE_CREATE: libc::c_ulong = 1;
const PR_SCHED_CORE_SCOPE_THREAD_GROUP: libc::c_ulong = 1;

/// Give this process a core scheduling cookie of its own, which every
/// thread started after shares: the kernel then never runs a thread of
/// another process on the sibling thread of a core one of these runs
/// on. `Ok(false)` when the host has no SMT or its kernel no core
/// scheduling: there is then nothing to keep apart.
pub fn own_core_cookie() -> io::Result<bool> {
    // SAFETY: the request takes plain integers; process 0 is the
    // caller's own.
    let result = unsafe {
        libc::prctl(
            PR_SCHED_CORE,
            PR_SCHED_CORE_CREATE,
            0,
            PR_SCHED_CORE_SCOPE_THREAD_GROUP,
            0,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENODEV | libc::EINVAL) => Ok(false),
        _ => Err(error),
    }
}

/// Keep the calling thread on the host thread `cpu`.
pub fn pin_this_thread(cpu: u32) -> io::Result<()> {
    let cpu = cpu as usize;
    // SAFETY: the set is a local of the type the calls take, zeroed
    // and then written by `CPU_SET` within its size, which is checked;
    // thread 0 is the caller's own.
    unsafe {
        let mut set = std::mem::zeroed::<libc::cpu_set_t>();
        if cpu >= 8 * std::mem::size_of::<libc::cpu_set_t>() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        libc::CPU_SET(cpu, &mut set);
        last_error(
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &raw const set),
            (),
        )
    }
}

/// The file the partition manager gave as descriptor `fd`, taken as it
/// is: a tap's file opened again by its `/proc/self/fd` path would be
/// another tap. It must be a character device.
pub fn inherited(fd: i32) -> io::Result<File> {
    if fd < 3 {
        return Err(io::Error::other(format!(
            "descriptor {fd} is not one given"
        )));
    }
    // SAFETY: F_GETFD reads nothing of memory; it says whether `fd` is
    // open.
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is open, as just seen, and is only borrowed for the
    // length of the duplication; nothing closes it meanwhile, the
    // process having no other thread yet.
    let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
    let file = File::from(borrowed.try_clone_to_owned()?);
    let kind = file.metadata()?.file_type();
    if !std::os::unix::fs::FileTypeExt::is_char_device(&kind) {
        return Err(io::Error::other(format!("descriptor {fd} is not a tap")));
    }
    Ok(file)
}

/// The connected Unix socket the partition manager gave as descriptor
/// `fd`: a passed tape drive's relay.
pub fn inherited_stream(fd: i32) -> io::Result<std::os::unix::net::UnixStream> {
    if fd < 3 {
        return Err(io::Error::other(format!(
            "descriptor {fd} is not one given"
        )));
    }
    // SAFETY: F_GETFD reads nothing of memory; it says whether `fd` is
    // open.
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is open, as just seen, and is only borrowed for the
    // length of the duplication; nothing closes it meanwhile, the
    // process having no other thread yet.
    let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
    let owned = borrowed.try_clone_to_owned()?;
    let kind = File::from(owned.try_clone()?).metadata()?.file_type();
    if !std::os::unix::fs::FileTypeExt::is_socket(&kind) {
        return Err(io::Error::other(format!("descriptor {fd} is not a socket")));
    }
    Ok(std::os::unix::net::UnixStream::from(owned))
}

/// The listening Unix socket the partition manager gave as descriptor
/// `fd`, taken as it is: a socket cannot be opened again by its path.
pub fn inherited_listener(fd: i32) -> io::Result<std::os::unix::net::UnixListener> {
    if fd < 3 {
        return Err(io::Error::other(format!(
            "descriptor {fd} is not one given"
        )));
    }
    // SAFETY: F_GETFD reads nothing of memory; it says whether `fd` is
    // open.
    if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is open, as just seen, and is only borrowed for the
    // length of the duplication; nothing closes it meanwhile, the
    // process having no other thread yet.
    let borrowed = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };
    let owned = borrowed.try_clone_to_owned()?;
    let kind = File::from(owned.try_clone()?).metadata()?.file_type();
    if !std::os::unix::fs::FileTypeExt::is_socket(&kind) {
        return Err(io::Error::other(format!("descriptor {fd} is not a socket")));
    }
    let listener = std::os::unix::net::UnixListener::from(owned);
    listener.set_nonblocking(false)?;
    Ok(listener)
}

/// Whether the process is root.
pub fn is_root() -> bool {
    // SAFETY: `geteuid` takes nothing and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// A processor's thread, by its handle, to be brought out of the guest
/// by a signal from another thread.
/// The handle is kept as std gives it: a number, which is a pointer
/// to musl and a number to glibc.
#[derive(Clone, Copy)]
pub struct Kick(pub std::os::unix::thread::RawPthread);

// SAFETY: the handle is of a processor's thread, taken while it was
// running; those threads live as long as the process does.
unsafe impl vmm_sys_util::signal::Killable for Kick {
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    fn pthread_handle(&self) -> libc::pthread_t {
        self.0 as libc::pthread_t
    }
}

/// Give the process namespaces of its own and an empty root, the log
/// directory bound in it (`base/confine`).
pub fn isolate(empty: &std::path::Path, log_dir: i32) -> io::Result<()> {
    confine::sys::isolate(empty, Some(log_dir))
}

/// Become the partition's user for good (`base/confine`).
pub fn drop_privileges(id: u32) -> io::Result<()> {
    confine::sys::drop_privileges(id)
}

/// Make the call `name` of the test hooks' list (`crate::hooks::CALLS`),
/// each one a VMM never needs: its return value, when the kernel lets
/// it return at all. Harmless if it were allowed: nothing it names
/// exists in the confined process's view, or it asks for nothing.
#[cfg(test_hooks)]
pub fn forbidden(name: &str) -> i64 {
    let path = c"/etc/passwd";
    // SAFETY: every call takes constants, null pointers or the string
    // above; the confinement ends the process before any takes effect,
    // and outside it each fails or does nothing to this process.
    unsafe {
        match name {
            "open" => i64::from(libc::open(path.as_ptr(), libc::O_RDONLY)),
            "socket" => i64::from(libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0)),
            "exec" => {
                let argv = [path.as_ptr(), std::ptr::null()];
                i64::from(libc::execve(path.as_ptr(), argv.as_ptr(), std::ptr::null()))
            }
            "fork" => libc::syscall(libc::SYS_fork),
            "ptrace" => libc::syscall(libc::SYS_ptrace, libc::PTRACE_TRACEME, 0, 0, 0),
            "setuid" => i64::from(libc::setuid(0)),
            "mount" => i64::from(libc::mount(
                std::ptr::null(),
                path.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null(),
            )),
            "kill" => i64::from(libc::kill(1, 0)),
            "unshare" => i64::from(libc::unshare(libc::CLONE_NEWUSER)),
            // An ioctl no policy names: the terminal's input queue.
            "ioctl" => i64::from(libc::ioctl(0, libc::TIOCSTI, path.as_ptr())),
            "bpf" => libc::syscall(libc::SYS_bpf, 0, 0, 0),
            "chroot" => i64::from(libc::chroot(path.as_ptr())),
            _ => -1,
        }
    }
}
