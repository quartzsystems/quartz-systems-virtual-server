// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvmm`, the virtual machine monitor: one process runs one
//! partition and nothing else.
//!
//! - [`profile`]: the partition's profile, a TOML file.
//! - [`end`]: why a partition ended, as the process's exit code and
//!   as a reference code.
//! - [`machine`]: the partition on KVM: memory, the interrupt
//!   controller, the devices, the processors.
//! - [`serial`]: where the 16550's output goes and its input comes
//!   from: the log and the console socket.
//! - [`run`]: the order of it all.
//!
//! The process starts as root, opens what it needs, and from then on
//! is an unprivileged user with no capability and no way to gain one
//! (`sys`). It reports on standard error: a reference code is a line
//! `REF <code> <text>`, which whoever started it posts.
//!
//! `unsafe` lives only in `sys`.

#![deny(unsafe_code)]

pub mod end;
pub mod profile;

#[cfg(target_os = "linux")]
pub mod machine;
#[cfg(target_os = "linux")]
pub mod run;
#[cfg(target_os = "linux")]
pub mod serial;
#[cfg(target_os = "linux")]
mod sys;

/// The user and group a partition runs as once it has opened its
/// devices. One for every partition until the partition manager gives
/// each its own.
pub const PARTITION_ID: u32 = 23_000;

/// Report a reference code on standard error.
pub fn post(code: &refcode::RefCode, detail: &str) {
    eprintln!("{} {} {}", refcode::WORD, code.code, code.text_with(detail));
}
