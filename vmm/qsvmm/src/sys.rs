// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The calls that need `unsafe`: giving KVM the guest's memory,
//! waiting for a signal, and dropping the process's privileges.

#![allow(unsafe_code)]

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

/// Become user and group `id` for good: no supplementary group, no
/// capability in any set, and no way to gain one through an exec.
/// Called while the process has one thread, so the thread's identity
/// is the process's.
pub fn drop_privileges(id: u32) -> io::Result<()> {
    // SAFETY: every call here takes plain integers, or a null pointer
    // with a zero count for the empty group list, and changes only the
    // calling process's credentials.
    unsafe {
        // The bounding set: every capability the kernel knows.
        for capability in 0..64 {
            if libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) != 0 {
                let error = io::Error::last_os_error();
                // Beyond the last capability the kernel has.
                if error.raw_os_error() == Some(libc::EINVAL) {
                    break;
                }
                return Err(error);
            }
        }
        last_error(libc::setgroups(0, std::ptr::null()), ())?;
        last_error(libc::setresgid(id, id, id), ())?;
        // Leaving user 0 clears the permitted, effective and ambient sets.
        last_error(libc::setresuid(id, id, id), ())?;
        last_error(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), ())?;
        // It must not be possible to go back.
        if libc::setresuid(0, 0, 0) == 0 || libc::getuid() != id || libc::geteuid() != id {
            return Err(io::Error::other("privileges were not dropped"));
        }
    }
    Ok(())
}

/// Whether the process is root.
pub fn is_root() -> bool {
    // SAFETY: `geteuid` takes nothing and cannot fail.
    unsafe { libc::geteuid() == 0 }
}
