// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A service confined: before it reads anything hostile, as root, the
//! process takes namespaces of its own and an empty, read-only root
//! (no network, no file system but what it was handed open), drops to
//! a user of its own with no capability, then installs a seccomp
//! filter of the calls it makes; a thread may narrow that to its own
//! kind's. The product gives each process's, and each thread kind's,
//! allowed calls as [`bpf::Rule`]s; any other call ends the process,
//! or in a learning mode is reported and fails.
//!
//! - [`bpf`]: the filter program from its rules.
//! - `sys` (Linux): the calls that do the confining.

#![deny(unsafe_code)]

pub mod bpf;
#[cfg(target_os = "linux")]
pub mod sys;

/// Confine the calling thread to `rules`: `default` (the program's
/// action for any other call, [`bpf::KILL_PROCESS`] or [`bpf::TRAP`] in
/// the learning mode) for the rest, and `kind` named for the report.
///
/// # Errors
/// The rules do not make a program, or the kernel refused it.
#[cfg(target_os = "linux")]
pub fn filter(kind: &'static str, rules: &[bpf::Rule], default: u32) -> std::io::Result<()> {
    let program = bpf::program(rules, default)
        .map_err(|e| std::io::Error::other(format!("the {kind} filter: {e}")))?;
    sys::install_filter(&program)?;
    sys::set_thread_kind(kind);
    Ok(())
}
