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

/// `SECCOMP_SET_MODE_FILTER`.
const SECCOMP_SET_MODE_FILTER: libc::c_ulong = 1;

/// Install `program` as a seccomp filter on the calling thread, which
/// the threads it starts after inherit. Filters stack: each one added
/// can only take away. `no_new_privs` is set first, as the kernel
/// requires of a process without `CAP_SYS_ADMIN`.
pub fn install_filter(program: &[crate::confine::bpf::Insn]) -> io::Result<()> {
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

/// `SIGSYS` in the learning mode (`crate::hooks`): the call was not
/// made; say which, on which kind of thread, and have it fail with
/// `ENOSYS`, so a whole run lists every call the policy lacks. Only
/// `write` is called, which every policy allows.
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
    put(b"qsvmm: confine: ", &mut at);
    put(crate::confine::thread_kind().as_bytes(), &mut at);
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

/// Have [`refused`] take `SIGSYS`: the learning mode's.
pub fn report_refused_calls() -> io::Result<()> {
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

/// Give the process namespaces of its own and an empty root, as root
/// and before the drop: a mount namespace whose root is an empty,
/// read-only `tmpfs` (no path names anything of the host's: no `/proc`,
/// no `/dev`, no other partition's files), a network namespace with
/// nothing but its own loopback, and IPC and UTS of its own. `empty` is
/// a directory the new root is mounted on; it is left empty on the
/// host. Everything the VMM uses is already open.
pub fn isolate(empty: &std::path::Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let at = CString::new(empty.as_os_str().as_bytes()).map_err(io::Error::other)?;
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
        // Nothing done here reaches the host's mounts.
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
        // The empty file system becomes the root; the old root, stacked
        // under it, is detached.
        if libc::syscall(libc::SYS_pivot_root, here.as_ptr(), here.as_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        last_error(libc::umount2(here.as_ptr(), libc::MNT_DETACH), ())?;
        last_error(libc::chdir(root.as_ptr()), ())?;
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
