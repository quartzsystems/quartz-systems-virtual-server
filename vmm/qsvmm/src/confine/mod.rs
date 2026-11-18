// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The VMM confined (`docs/SECURITY.md`). Before the privilege drop, as
//! root, the process takes namespaces of its own and an empty root
//! (`sys::isolate`): no network, no file system, no other process to
//! see. After the drop, with every descriptor in hand, a seccomp filter
//! of every call any of its threads makes is installed on the main
//! thread before it starts one ([`Kind::Process`]); then each thread
//! narrows that to its own kind's calls as it starts ([`enter`]).
//! Filters stack, so a thread can only lose calls. Any other call ends
//! the whole process (`SIGSYS`), which the partition manager posts as
//! `A2005190`.
//!
//! In a test image the hook `confinelearn` has a refused call reported
//! and failed with `ENOSYS` instead (`sys::report_refused_calls`), so a
//! run lists what the policy lacks.
//!
//! - [`bpf`]: the filter program from its rules.
//! - [`policy`]: what each kind of thread may call.

pub mod bpf;
pub mod policy;

use std::cell::Cell;
use std::io;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// A kind of thread, with its own calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The process, before any thread starts: every kind's calls.
    Process,
    /// The main thread once everything is started: it waits for the
    /// end and says why.
    Main,
    /// A processor.
    Processor,
    /// A virtio device's worker.
    Worker,
    /// The serial log and the console's clients.
    Serial,
    /// The console socket's listener.
    Console,
    /// A console client's input.
    ConsoleIn,
    /// The control socket.
    Control,
    /// A service channel port's carrier.
    Channel,
    /// The drives' watch.
    Drives,
    /// The signals' thread.
    Signals,
}

impl Kind {
    /// Its name, in what the learning mode reports.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Process => "process",
            Kind::Main => "main",
            Kind::Processor => "processor",
            Kind::Worker => "worker",
            Kind::Serial => "serial",
            Kind::Console => "console",
            Kind::ConsoleIn => "console-in",
            Kind::Control => "control",
            Kind::Channel => "channel",
            Kind::Drives => "drives",
            Kind::Signals => "signals",
        }
    }
}

thread_local! {
    static KIND: Cell<&'static str> = const { Cell::new("unconfined") };
}

/// The kind of the calling thread, for the learning mode's report.
pub fn thread_kind() -> &'static str {
    KIND.with(Cell::get)
}

/// What the filters are built against: the process's ID (a signal may
/// go only to its own threads) and the log directory's descriptor (the
/// only one a new file may be opened in).
static PROCESS: AtomicU32 = AtomicU32::new(0);
static LOG_DIR: AtomicI32 = AtomicI32::new(-1);

/// Note the log directory's descriptor, before the filters are built.
pub fn log_dir(fd: i32) {
    LOG_DIR.store(fd, Ordering::Relaxed);
}

/// Whether refused calls are reported and failed, not fatal.
fn learning() -> bool {
    crate::hooks::confine_learn()
}

/// Confine the calling thread to `kind`'s calls.
pub fn enter(kind: Kind) -> io::Result<()> {
    // A thread narrows the process's filter: before the process is
    // confined (a unit test's threads) there is nothing to narrow.
    if kind != Kind::Process && PROCESS.load(Ordering::Relaxed) == 0 {
        return Ok(());
    }
    if kind == Kind::Process {
        PROCESS.store(std::process::id(), Ordering::Relaxed);
        if learning() {
            crate::sys::report_refused_calls()?;
        }
    }
    let context = policy::Context {
        process: PROCESS.load(Ordering::Relaxed),
        log_dir: LOG_DIR.load(Ordering::Relaxed),
        kick: u32::try_from(crate::machine::vcpu::kick_signal()).unwrap_or(0),
    };
    let default = if learning() {
        bpf::TRAP
    } else {
        bpf::KILL_PROCESS
    };
    let program = bpf::program(&policy::rules(kind, &context), default)
        .map_err(|e| io::Error::other(format!("the {} filter: {e}", kind.name())))?;
    crate::sys::install_filter(&program)?;
    KIND.with(|k| k.set(kind.name()));
    Ok(())
}

/// [`enter`] from a thread that cannot say why it failed: a thread
/// that cannot be confined ends the process.
pub fn enter_or_end(kind: Kind) {
    if let Err(e) = enter(kind) {
        eprintln!("qsvmm: confine: {}: {e}", kind.name());
        std::process::exit(40);
    }
}
