// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What each kind of thread may call, once every descriptor is in hand:
//! taken from what the VMM's threads were seen to call under `strace`
//! through a partition's whole life (`docs/SECURITY.md`) and from the
//! paths a test run in the learning mode reached. Every thread has the
//! runtime's calls ([`COMMON`]); each kind adds its own. `ioctl` is
//! allowed only for the requests named, `clone` only for a thread,
//! a signal only to the process's own threads, and a file opened or
//! moved only in the serial log's directory.

use super::Kind;
use super::bpf::Rule;

/// What the rules are built against.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    /// The process's ID.
    pub process: u32,
    /// The serial log directory's descriptor; -1 when there is none.
    pub log_dir: i32,
    /// The signal that brings a processor out of the guest.
    pub kick: u32,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // the ABI's numbers
const fn nr(n: libc::c_long) -> u32 {
    n as u32
}

/// The flags of a thread's `clone`, as musl's `pthread_create` gives
/// them: nothing else may be cloned.
pub const THREAD_FLAGS: u32 = 0x007D_0F00;

/// Every thread's: memory, futexes, signals' return, the clock, time,
/// thread exit, writing what it says to standard error.
const COMMON: &[libc::c_long] = &[
    libc::SYS_read,
    libc::SYS_write,
    libc::SYS_writev,
    libc::SYS_close,
    libc::SYS_futex,
    libc::SYS_mmap,
    libc::SYS_munmap,
    libc::SYS_mremap,
    libc::SYS_mprotect,
    libc::SYS_madvise,
    libc::SYS_brk,
    libc::SYS_rt_sigprocmask,
    libc::SYS_rt_sigreturn,
    libc::SYS_sigaltstack,
    libc::SYS_sched_yield,
    libc::SYS_nanosleep,
    libc::SYS_clock_nanosleep,
    libc::SYS_clock_gettime,
    libc::SYS_gettimeofday,
    libc::SYS_gettid,
    libc::SYS_getpid,
    libc::SYS_getrandom,
    libc::SYS_restart_syscall,
    libc::SYS_exit,
    libc::SYS_exit_group,
    libc::SYS_fcntl,
    libc::SYS_lseek,
    libc::SYS_fstat,
    libc::SYS_newfstatat,
    libc::SYS_statx,
    libc::SYS_poll,
    libc::SYS_ppoll,
];

/// `prctl`'s requests a thread makes: its name, and no new privileges
/// (set again with each filter).
const PRCTL: &[u32] = &[15, 16, 38];
/// `SECCOMP_SET_MODE_FILTER`: a filter added can only take away.
const ADD_FILTER: u32 = 1;

/// The KVM requests of a processor's thread: running it, its state
/// for a dump or a report, its clock after a hold, and what a guest's
/// MSI and MSI-X programming asks of the interrupt routing.
const KVM: &[u32] = &[
    0xAE80,      // KVM_RUN
    0x8090_AE81, // KVM_GET_REGS
    0x4090_AE82, // KVM_SET_REGS
    0x8138_AE83, // KVM_GET_SREGS
    0xC018_AE85, // KVM_TRANSLATE
    0xC008_AE88, // KVM_GET_MSRS
    0x81A0_AE8C, // KVM_GET_FPU
    0x8400_AE8E, // KVM_GET_LAPIC
    0x8004_AE98, // KVM_GET_MP_STATE
    0x8040_AE9F, // KVM_GET_VCPU_EVENTS
    0x8080_AEA1, // KVM_GET_DEBUGREGS
    0x9000_AEA4, // KVM_GET_XSAVE
    0x8188_AEA6, // KVM_GET_XCRS
    0xAEAD,      // KVM_KVMCLOCK_CTRL
    0x4008_AE6A, // KVM_SET_GSI_ROUTING
    0x4020_AE76, // KVM_IRQFD
    0x4040_AE79, // KVM_IOEVENTFD
    0x4020_AEA5, // KVM_SIGNAL_MSI
];

/// What a virtio-net adapter's start and reset ask of vhost-net and
/// the tap, on the processor that wrote the device's status. The owner
/// and the memory table are set at the adapter's first start, which is
/// the guest's.
const VHOST: &[u32] = &[
    0xAF01,      // VHOST_SET_OWNER
    0x4008_AF03, // VHOST_SET_MEM_TABLE
    0x8008_AF00, // VHOST_GET_FEATURES
    0x4008_AF00, // VHOST_SET_FEATURES
    0x4008_AF10, // VHOST_SET_VRING_NUM
    0x4028_AF11, // VHOST_SET_VRING_ADDR
    0x4008_AF12, // VHOST_SET_VRING_BASE
    0xC008_AF12, // VHOST_GET_VRING_BASE
    0x4008_AF20, // VHOST_SET_VRING_KICK
    0x4008_AF21, // VHOST_SET_VRING_CALL
    0x4008_AF30, // VHOST_NET_SET_BACKEND
    0x4004_54D0, // TUNSETOFFLOAD
];

/// What a passed function's interrupts ask of VFIO when the guest
/// programs them.
const VFIO: &[u32] = &[
    0x3B6E, // VFIO_DEVICE_SET_IRQS
];

/// The descriptor requests the runtime makes.
const DESCRIPTORS: &[u32] = &[
    0x5421, // FIONBIO
    0x5451, // FIOCLEX
];

/// A processor: KVM, the devices it reaches through its exits (a
/// virtio device's start makes its worker, its eventfds and epoll), a
/// passed function's regions, and a hold's kick of the others.
const PROCESSOR: &[libc::c_long] = &[
    libc::SYS_eventfd2,
    libc::SYS_epoll_create1,
    libc::SYS_epoll_ctl,
    libc::SYS_pread64,
    libc::SYS_pwrite64,
    libc::SYS_dup,
    libc::SYS_sched_getaffinity,
];

/// A device's worker: its queues' notifications, its backing stores,
/// and the relay of a passed tape drive.
const WORKER: &[libc::c_long] = &[
    libc::SYS_epoll_wait,
    libc::SYS_epoll_pwait,
    libc::SYS_pread64,
    libc::SYS_pwrite64,
    libc::SYS_preadv,
    libc::SYS_pwritev,
    libc::SYS_fsync,
    libc::SYS_fdatasync,
    libc::SYS_fallocate,
    libc::SYS_ftruncate,
    libc::SYS_sendmsg,
    libc::SYS_recvmsg,
    libc::SYS_sendto,
    libc::SYS_recvfrom,
];

/// The serial log and the console's clients: a new generation of the
/// log, in its directory only.
const SERIAL: &[libc::c_long] = &[libc::SYS_fdatasync, libc::SYS_fsync, libc::SYS_sendto];

/// A listener: the console's, the control socket's, a port's.
const LISTENER: &[libc::c_long] = &[
    libc::SYS_accept4,
    libc::SYS_accept,
    libc::SYS_recvfrom,
    libc::SYS_recvmsg,
    libc::SYS_sendto,
    libc::SYS_sendmsg,
    libc::SYS_setsockopt,
    libc::SYS_getsockopt,
    libc::SYS_shutdown,
];

/// The control socket: a dump written into the file it was given, a
/// medium loaded, the processors kicked to hold.
const CONTROL: &[libc::c_long] = &[
    libc::SYS_pread64,
    libc::SYS_pwrite64,
    libc::SYS_pwritev,
    libc::SYS_ftruncate,
    libc::SYS_fallocate,
    libc::SYS_fsync,
    libc::SYS_fdatasync,
];

/// The drives' watch: what a volume's notices need.
const DRIVES: &[libc::c_long] = &[
    libc::SYS_pread64,
    libc::SYS_pwrite64,
    libc::SYS_fsync,
    libc::SYS_fdatasync,
];

/// The signals' thread: waiting for one.
const SIGNALS: &[libc::c_long] = &[libc::SYS_rt_sigtimedwait];

/// The main thread: the end's drain and what it says.
const MAIN: &[libc::c_long] = &[libc::SYS_fsync, libc::SYS_fdatasync];

fn calls(kind: Kind) -> Vec<libc::c_long> {
    let own: &[libc::c_long] = match kind {
        Kind::Process | Kind::Control => &[],
        Kind::Main => MAIN,
        Kind::Processor => PROCESSOR,
        Kind::Worker => WORKER,
        Kind::Serial => SERIAL,
        Kind::Console | Kind::ConsoleIn | Kind::Channel => LISTENER,
        Kind::Drives => DRIVES,
        Kind::Signals => SIGNALS,
    };
    let mut out: Vec<libc::c_long> = COMMON.to_vec();
    out.extend_from_slice(own);
    match kind {
        Kind::Control => {
            out.extend_from_slice(LISTENER);
            out.extend_from_slice(CONTROL);
        }
        // A filter a thread was started under stays under its own: a
        // kind that starts threads has their calls too.
        Kind::Processor => out.extend_from_slice(WORKER),
        _ => {}
    }
    out
}

/// The kinds that start threads: the main thread (before it narrows),
/// a processor (a device's worker, when the guest starts the device),
/// and the console's listener (a client's input).
fn starts_threads(kind: Kind) -> bool {
    matches!(kind, Kind::Processor | Kind::Console | Kind::Channel)
}

/// The kinds that kick other threads of the process. A device's
/// worker holds the processors when its pool has no room for a write.
fn kicks(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Processor | Kind::Worker | Kind::Control | Kind::Signals
    )
}

fn ioctls(kind: Kind) -> Vec<u32> {
    let mut out = DESCRIPTORS.to_vec();
    if kind == Kind::Processor {
        out.extend_from_slice(KVM);
        out.extend_from_slice(VHOST);
        out.extend_from_slice(VFIO);
    }
    if kind == Kind::Control {
        // A dump reads each processor's state through KVM.
        out.extend_from_slice(KVM);
    }
    out
}

/// The kinds every other one is a part of.
const ALL: [Kind; 10] = [
    Kind::Main,
    Kind::Processor,
    Kind::Worker,
    Kind::Serial,
    Kind::Console,
    Kind::ConsoleIn,
    Kind::Control,
    Kind::Channel,
    Kind::Drives,
    Kind::Signals,
];

/// The rules of `kind`. The process's are every kind's together, with
/// what the main thread does between the drop and its own narrowing:
/// signal handlers installed, threads started.
pub fn rules(kind: Kind, cx: &Context) -> Vec<Rule> {
    let kinds: Vec<Kind> = if kind == Kind::Process {
        ALL.to_vec()
    } else {
        vec![kind]
    };
    let mut plain: Vec<u32> = Vec::new();
    let mut requests: Vec<u32> = Vec::new();
    let mut threads = false;
    let mut kick = false;
    for k in &kinds {
        for c in calls(*k) {
            let n = nr(c);
            if !plain.contains(&n) {
                plain.push(n);
            }
        }
        for r in ioctls(*k) {
            if !requests.contains(&r) {
                requests.push(r);
            }
        }
        threads |= starts_threads(*k);
        kick |= kicks(*k);
    }
    if kind == Kind::Process {
        threads = true;
        plain.push(nr(libc::SYS_rt_sigaction));
        plain.push(nr(libc::SYS_sched_setaffinity));
        plain.push(nr(libc::SYS_set_robust_list));
        plain.push(nr(libc::SYS_rseq));
    }
    plain.sort_unstable();
    let mut out: Vec<Rule> = plain.into_iter().map(Rule::Any).collect();
    out.push(Rule::When(nr(libc::SYS_ioctl), 1, requests));
    out.push(Rule::When(nr(libc::SYS_prctl), 0, PRCTL.to_vec()));
    if threads {
        // Each thread narrows the filter it was started under with its
        // own: the process's, or the kind's that started it.
        out.push(Rule::When(nr(libc::SYS_seccomp), 0, vec![ADD_FILTER]));
    }
    if threads {
        out.push(Rule::When(nr(libc::SYS_clone), 0, vec![THREAD_FLAGS]));
    }
    if kick {
        // Only to a thread of this process.
        out.push(Rule::When(nr(libc::SYS_tgkill), 0, vec![cx.process]));
        // musl's pthread_kill: the kick alone. Its target is not
        // checked here; another process is not this user's, and the
        // kernel refuses it.
        out.push(Rule::When(nr(libc::SYS_tkill), 1, vec![cx.kick]));
    }
    if (kind == Kind::Serial || kind == Kind::Process) && cx.log_dir >= 0 {
        #[allow(clippy::cast_sign_loss)] // not negative, above
        let dir = cx.log_dir as u32;
        out.push(Rule::When(nr(libc::SYS_openat), 0, vec![dir]));
        out.push(Rule::When(nr(libc::SYS_renameat), 0, vec![dir]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::bpf::{ALLOW, ARCH_X86_64, KILL_PROCESS, program, run};
    use super::*;

    fn decide(kind: Kind, call: libc::c_long, args: [u64; 6]) -> u32 {
        let cx = Context {
            process: 4242,
            log_dir: 7,
            kick: 35,
        };
        let p = program(&rules(kind, &cx), KILL_PROCESS).unwrap();
        run(&p, ARCH_X86_64, nr(call), args)
    }

    const FORBIDDEN: &[libc::c_long] = &[
        libc::SYS_open,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_fork,
        libc::SYS_vfork,
        libc::SYS_clone3,
        libc::SYS_socket,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_ptrace,
        libc::SYS_setuid,
        libc::SYS_setresuid,
        libc::SYS_capset,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_kill,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_seccomp,
        libc::SYS_keyctl,
        libc::SYS_init_module,
        libc::SYS_reboot,
        libc::SYS_mknodat,
        libc::SYS_unlinkat,
        libc::SYS_symlinkat,
        libc::SYS_io_uring_setup,
        libc::SYS_userfaultfd,
        libc::SYS_memfd_create,
    ];

    #[test]
    fn no_thread_may_make_a_call_a_vmm_never_needs() {
        for kind in ALL.iter().copied().chain([Kind::Process]) {
            for call in FORBIDDEN {
                if (starts_threads(kind) || kind == Kind::Process) && *call == libc::SYS_seccomp {
                    continue;
                }
                assert_eq!(
                    decide(kind, *call, [0; 6]),
                    KILL_PROCESS,
                    "{} may call {call}",
                    kind.name()
                );
            }
        }
    }

    #[test]
    fn a_thread_is_cloned_and_a_process_is_not() {
        let clone = |flags| decide(Kind::Processor, libc::SYS_clone, [flags, 0, 0, 0, 0, 0]);
        assert_eq!(clone(u64::from(THREAD_FLAGS)), ALLOW);
        // A process: no CLONE_THREAD, no CLONE_VM.
        assert_eq!(clone(0x11), KILL_PROCESS);
        assert_eq!(clone(u64::from(THREAD_FLAGS) | 0x2000_0000), KILL_PROCESS);
        assert_eq!(
            decide(
                Kind::Worker,
                libc::SYS_clone,
                [u64::from(THREAD_FLAGS), 0, 0, 0, 0, 0]
            ),
            KILL_PROCESS
        );
    }

    #[test]
    fn a_signal_goes_only_to_this_process() {
        let kick = |pid| decide(Kind::Processor, libc::SYS_tgkill, [pid, 7, 34, 0, 0, 0]);
        assert_eq!(kick(4242), ALLOW);
        assert_eq!(kick(1), KILL_PROCESS);
        assert_eq!(
            decide(Kind::Serial, libc::SYS_tgkill, [4242, 7, 34, 0, 0, 0]),
            KILL_PROCESS
        );
        // A worker holds the processors for its pool's space.
        assert_eq!(
            decide(Kind::Worker, libc::SYS_tkill, [99, 35, 0, 0, 0, 0]),
            ALLOW
        );
        assert_eq!(
            decide(Kind::Worker, libc::SYS_tgkill, [1, 7, 35, 0, 0, 0]),
            KILL_PROCESS
        );
        let tkill = |signal| decide(Kind::Control, libc::SYS_tkill, [99, signal, 0, 0, 0, 0]);
        assert_eq!(tkill(35), ALLOW);
        assert_eq!(tkill(9), KILL_PROCESS);
        assert_eq!(tkill(19), KILL_PROCESS);
    }

    #[test]
    fn only_the_serial_log_opens_a_file_and_only_in_its_directory() {
        let open = |kind, dir| decide(kind, libc::SYS_openat, [dir, 0, 0, 0, 0, 0]);
        assert_eq!(open(Kind::Serial, 7), ALLOW);
        // AT_FDCWD, or any other directory.
        assert_eq!(open(Kind::Serial, 0xFFFF_FF9C), KILL_PROCESS);
        assert_eq!(open(Kind::Serial, 3), KILL_PROCESS);
        assert_eq!(open(Kind::Processor, 7), KILL_PROCESS);
        assert_eq!(open(Kind::Control, 7), KILL_PROCESS);
    }

    #[test]
    fn kvm_is_the_processors_and_a_dump_s() {
        let ioctl =
            |kind, request: u32| decide(kind, libc::SYS_ioctl, [5, u64::from(request), 0, 0, 0, 0]);
        assert_eq!(ioctl(Kind::Processor, 0xAE80), ALLOW);
        assert_eq!(ioctl(Kind::Worker, 0xAE80), KILL_PROCESS);
        // KVM_CREATE_VM, KVM_SET_USER_MEMORY_REGION: no one, after start.
        assert_eq!(ioctl(Kind::Processor, 0xAE01), KILL_PROCESS);
        assert_eq!(ioctl(Kind::Process, 0x4020_AE46), KILL_PROCESS);
        assert_eq!(ioctl(Kind::Control, 0x8090_AE81), ALLOW);
        assert_eq!(ioctl(Kind::Serial, 0x5421), ALLOW);
        // An adapter's first start: its owner and the memory table.
        assert_eq!(ioctl(Kind::Processor, 0xAF01), ALLOW);
        assert_eq!(ioctl(Kind::Processor, 0x4008_AF03), ALLOW);
        assert_eq!(ioctl(Kind::Worker, 0xAF01), KILL_PROCESS);
    }

    #[test]
    fn a_filter_is_added_only_before_a_thread_narrows_and_never_taken_away() {
        let filter = |kind, op| decide(kind, libc::SYS_seccomp, [op, 0, 0, 0, 0, 0]);
        assert_eq!(filter(Kind::Process, 1), ALLOW);
        // SECCOMP_SET_MODE_STRICT, SECCOMP_GET_ACTION_AVAIL: no.
        assert_eq!(filter(Kind::Process, 0), KILL_PROCESS);
        assert_eq!(filter(Kind::Process, 2), KILL_PROCESS);
        assert_eq!(filter(Kind::Worker, 1), KILL_PROCESS);
        // A processor starts a device's worker, which narrows in its turn.
        assert_eq!(filter(Kind::Processor, 1), ALLOW);
        let prctl = |op| decide(Kind::Worker, libc::SYS_prctl, [op, 0, 0, 0, 0, 0]);
        assert_eq!(prctl(15), ALLOW);
        // PR_SET_DUMPABLE, PR_SET_MM, PR_SET_PTRACER: no.
        for op in [4, 35, 0x5961_6d61] {
            assert_eq!(prctl(op), KILL_PROCESS);
        }
    }

    #[test]
    fn the_process_has_every_kind_s_calls() {
        for kind in ALL {
            for call in calls(kind) {
                assert_eq!(
                    decide(Kind::Process, call, [0x7D_0F00, 0, 0, 0, 0, 0]),
                    ALLOW,
                    "{call} of {}",
                    kind.name()
                );
            }
        }
    }
}
