// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The calls that confine a process: namespaces and an empty root, the
//! drop to a user with no capability, a seccomp filter, and the
//! learning mode's report of a refused call. `unsafe` lives only here.

#![allow(unsafe_code)] // every block says why it is sound

use std::cell::Cell;
use std::ffi::CStr;
use std::io;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::bpf::Insn;

fn last_error<T>(result: libc::c_int, value: T) -> io::Result<T> {
    if result == 0 {
        Ok(value)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `SECCOMP_SET_MODE_FILTER`.
const SECCOMP_SET_MODE_FILTER: libc::c_ulong = 1;

/// Install `program` as a seccomp filter on the calling thread, which
/// the threads it starts after inherit. Filters stack: each one added
/// can only take away. `no_new_privs` is set first, as the kernel
/// requires of a process without `CAP_SYS_ADMIN`.
pub fn install_filter(program: &[Insn]) -> io::Result<()> {
    let len = u16::try_from(program.len()).map_err(io::Error::other)?;
    let fprog = libc::sock_fprog {
        len,
        // The kernel reads the program, it does not write it.
        filter: program.as_ptr().cast::<libc::sock_filter>().cast_mut(),
    };
    // SAFETY: `Insn` is `repr(C)` with `sock_filter`'s layout (a test
    // holds it); `fprog` points at `program` for the call's length; the
    // kernel copies the program before it returns.
    unsafe {
        last_error(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0), ())?;
        let r = libc::syscall(
            libc::SYS_seccomp,
            SECCOMP_SET_MODE_FILTER,
            0 as libc::c_ulong,
            &raw const fprog,
        );
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
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

thread_local! {
    static KIND: Cell<&'static str> = const { Cell::new("unconfined") };
}

/// Say what kind of thread the calling one is, for the learning mode's
/// report.
pub fn set_thread_kind(kind: &'static str) {
    KIND.with(|k| k.set(kind));
}

/// The kind of the calling thread.
pub fn thread_kind() -> &'static str {
    KIND.with(Cell::get)
}

/// What starts a report line: the process's name and `: confine: `.
static WHO: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static WHO_LEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The digits of `n` in `base` into `out`, as a signal handler may: no
/// allocation. The bytes written.
fn digits(mut n: u64, base: u64, out: &mut [u8]) -> usize {
    let mut tmp = [0u8; 20];
    let mut i = 0;
    loop {
        // A digit of the base: below 16.
        #[allow(clippy::cast_possible_truncation)]
        let d = (n % base) as u8;
        tmp[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        i += 1;
        n /= base;
        if n == 0 || i == tmp.len() {
            break;
        }
    }
    let n = i.min(out.len());
    for (j, b) in out.iter_mut().take(n).enumerate() {
        *b = tmp[i - 1 - j];
    }
    n
}

/// `SIGSYS` in the learning mode: the call was not made; say which, on
/// which kind of thread, and have it fail with `ENOSYS`, so a whole run
/// lists every call the policy lacks. Only `write` is called, which
/// every policy allows.
extern "C" fn refused(_: libc::c_int, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
    // SAFETY: the kernel gives a SIGSYS handler installed with
    // SA_SIGINFO its siginfo and its ucontext; for SIGSYS the call's
    // number is the int at offset 24 of siginfo (`_sigsys._syscall`).
    let (nr, args, gregs) = unsafe {
        let nr = info.cast::<u8>().add(24).cast::<i32>().read_unaligned();
        let uc = context.cast::<libc::ucontext_t>();
        let g = &mut (*uc).uc_mcontext.gregs;
        let args = [
            g[libc::REG_RDI as usize],
            g[libc::REG_RSI as usize],
            g[libc::REG_RDX as usize],
        ];
        (nr, args, g)
    };
    let mut line = [0u8; 160];
    let mut at = 0;
    let mut put = |bytes: &[u8], at: &mut usize| {
        let n = bytes.len().min(line.len() - *at);
        line[*at..*at + n].copy_from_slice(&bytes[..n]);
        *at += n;
    };
    let who = WHO.load(Ordering::Relaxed);
    if !who.is_null() {
        // SAFETY: set once from a `&'static str` by
        // `report_refused_calls`, with its length.
        let s = unsafe { std::slice::from_raw_parts(who, WHO_LEN.load(Ordering::Relaxed)) };
        put(s, &mut at);
    }
    put(b": confine: ", &mut at);
    put(thread_kind().as_bytes(), &mut at);
    put(b" made system call ", &mut at);
    let mut num = [0u8; 20];
    let n = digits(u64::try_from(nr).unwrap_or(0), 10, &mut num);
    put(&num[..n], &mut at);
    for a in args {
        put(b" 0x", &mut at);
        #[allow(clippy::cast_sign_loss)] // a register's bits
        let n = digits(a as u64, 16, &mut num);
        put(&num[..n], &mut at);
    }
    put(b"\n", &mut at);
    // SAFETY: `write` of a buffer this function owns, to standard error;
    // the saved RAX is the interrupted call's return value.
    unsafe {
        libc::write(2, line.as_ptr().cast(), at);
        gregs[libc::REG_RAX as usize] = -i64::from(libc::ENOSYS);
    }
}

/// The learning mode: a call a filter of `SECCOMP_RET_TRAP` refuses is
/// reported on standard error as `<who>: confine: <kind> made system
/// call N 0x.. 0x.. 0x..` and fails with `ENOSYS`.
pub fn report_refused_calls(who: &'static str) -> io::Result<()> {
    WHO_LEN.store(who.len(), Ordering::Relaxed);
    WHO.store(who.as_ptr().cast_mut(), Ordering::Relaxed);
    // SAFETY: a zeroed sigaction with a handler of the SA_SIGINFO form
    // and an empty mask; the handler calls only `write`.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = refused as *const () as usize;
        action.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&raw mut action.sa_mask);
        last_error(
            libc::sigaction(libc::SIGSYS, &raw const action, std::ptr::null_mut()),
            (),
        )
    }
}

/// Bind the directory at `source` at `log` in the current directory,
/// the new root before it is moved: with no set-user-ID, no device and
/// nothing to execute.
fn bind_log(source: &CStr) -> io::Result<()> {
    let log = c"log";
    // SAFETY: NUL-terminated strings that live across each call, or null
    // pointers where the kernel allows them.
    unsafe {
        last_error(libc::mkdir(log.as_ptr(), 0o555), ())?;
        last_error(
            libc::mount(
                source.as_ptr(),
                log.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            ),
            (),
        )?;
        last_error(
            libc::mount(
                std::ptr::null(),
                log.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND
                    | libc::MS_REMOUNT
                    | libc::MS_NOSUID
                    | libc::MS_NODEV
                    | libc::MS_NOEXEC,
                std::ptr::null(),
            ),
            (),
        )
    }
}

/// Replace the descriptor `log_dir` by `/log` opened in the new root:
/// the same number, which the caller holds.
fn reopen_log(log_dir: i32) -> io::Result<()> {
    // SAFETY: a NUL-terminated path; descriptors this process owns.
    unsafe {
        let inside = libc::open(
            c"/log".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        );
        if inside < 0 {
            return Err(io::Error::last_os_error());
        }
        let moved = libc::dup3(inside, log_dir, libc::O_CLOEXEC);
        let error = io::Error::last_os_error();
        libc::close(inside);
        if moved < 0 {
            return Err(error);
        }
    }
    Ok(())
}

/// Give the process namespaces of its own and an empty root, as root
/// and before the drop: a mount namespace whose root is an empty,
/// read-only `tmpfs` (no path names anything of the machine's: no
/// `/proc`, no `/dev`, no other service's files), a network namespace
/// with nothing but its own loopback, and IPC and UTS of its own.
/// `empty` is a directory the new root is mounted on; it is left empty
/// outside. Everything the process uses is open already: it keeps only
/// what it was handed.
///
/// A directory it writes, `log_dir` (an open descriptor), when given,
/// is bound at `/log` in the new root and the descriptor replaced by
/// one opened there, so that a name resolved against it (`..` too)
/// stays in the empty root.
pub fn isolate(empty: &std::path::Path, log_dir: Option<i32>) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let at = CString::new(empty.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let log_path = match log_dir {
        Some(fd) => {
            let p = std::fs::read_link(format!("/proc/self/fd/{fd}"))?;
            Some(CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other)?)
        }
        None => None,
    };
    let tmpfs = c"tmpfs";
    let none = c"none";
    let root = c"/";
    let here = c".";
    let options = c"size=4k,nr_inodes=4,mode=0555";
    // SAFETY: each call takes NUL-terminated strings that live across
    // it, or null pointers where the kernel allows them, and changes
    // only this process's namespaces and its view of the file system.
    unsafe {
        last_error(
            libc::unshare(
                libc::CLONE_NEWNS | libc::CLONE_NEWNET | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS,
            ),
            (),
        )?;
        // Nothing done here reaches the machine's mounts.
        last_error(
            libc::mount(
                std::ptr::null(),
                root.as_ptr(),
                std::ptr::null(),
                libc::MS_REC | libc::MS_PRIVATE,
                std::ptr::null(),
            ),
            (),
        )?;
        last_error(
            libc::mount(
                none.as_ptr(),
                at.as_ptr(),
                tmpfs.as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                options.as_ptr().cast(),
            ),
            (),
        )?;
        last_error(libc::chdir(at.as_ptr()), ())?;
    }
    if let Some(p) = &log_path {
        bind_log(p)?;
    }
    // SAFETY: as above.
    unsafe {
        // The empty file system becomes the root, as switch_root leaves
        // an initramfs (which pivot_root refuses to leave): moved over
        // `/` in this namespace, and entered. Nothing after the drop may
        // mount or chroot again.
        last_error(
            libc::mount(
                here.as_ptr(),
                root.as_ptr(),
                std::ptr::null(),
                libc::MS_MOVE,
                std::ptr::null(),
            ),
            (),
        )?;
        last_error(libc::chroot(here.as_ptr()), ())?;
        last_error(libc::chdir(root.as_ptr()), ())?;
    }
    if let Some(fd) = log_dir {
        reopen_log(fd)?;
    }
    // SAFETY: as above.
    unsafe {
        last_error(
            libc::mount(
                std::ptr::null(),
                root.as_ptr(),
                std::ptr::null(),
                libc::MS_REMOUNT
                    | libc::MS_RDONLY
                    | libc::MS_NOSUID
                    | libc::MS_NODEV
                    | libc::MS_NOEXEC,
                std::ptr::null(),
            ),
            (),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn digits_are_written_without_allocating() {
        let mut out = [0u8; 20];
        let n = super::digits(255, 16, &mut out);
        assert_eq!(&out[..n], b"ff");
        let n = super::digits(0, 10, &mut out);
        assert_eq!(&out[..n], b"0");
    }

    #[test]
    fn an_insn_is_a_sock_filter() {
        assert_eq!(
            std::mem::size_of::<crate::bpf::Insn>(),
            std::mem::size_of::<libc::sock_filter>()
        );
    }
}
