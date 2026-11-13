// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvmm`, the virtual machine monitor: one process runs one
//! partition and nothing else.
//!
//! - [`config`]: the partition's configuration for one activation,
//!   which the partition manager writes to standard input.
//! - [`end`]: why a partition ended, as the process's exit code and
//!   as a reference code.
//! - [`machine`]: the partition on KVM: memory, the interrupt
//!   controller, the devices, the processors.
//! - [`serial`]: where the 16550's output goes and its input comes
//!   from: the log and the console socket.
//! - [`run`]: the order of it all.
//!
//! The process starts as root, opens what it needs, and from then on
//! is the partition's own unprivileged user with no capability and no
//! way to gain one (`sys`). It reports on standard error: a reference code is a line
//! `REF <code> <text>`, which whoever started it posts.
//!
//! `unsafe` lives only in `sys`.

#![deny(unsafe_code)]

pub mod config;
pub mod end;

#[cfg(target_os = "linux")]
pub mod machine;
#[cfg(target_os = "linux")]
pub mod run;
#[cfg(target_os = "linux")]
pub mod serial;
#[cfg(target_os = "linux")]
mod sys;

use std::path::{Path, PathBuf};

/// The partition `id`'s log directory under the data directory:
/// `log/partitions/<id>`. The serial log is in it.
pub fn log_dir(data_dir: &Path, id: u32) -> PathBuf {
    data_dir.join("log").join("partitions").join(id.to_string())
}

/// The partition `id`'s directory under the run directory:
/// `partitions/<id>`. The console socket is in it.
pub fn socket_dir(run_dir: &Path, id: u32) -> PathBuf {
    run_dir.join("partitions").join(id.to_string())
}

/// The file name of the serial log in a partition's log directory.
pub const SERIAL_LOG: &str = "serial.log";
/// The file name of the console socket in a partition's directory.
pub const CONSOLE_SOCKET: &str = "console.sock";

/// The word that starts the last line of a report: `END <exit code>`.
/// Whoever was not there to wait for the process reads how it ended
/// from this.
pub const END_WORD: &str = "END";

/// The partition's name, once its configuration is read: every code
/// reported after that names it.
static PARTITION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Name the partition this process runs.
pub fn name_partition(name: &str) {
    let _ = PARTITION.set(name.to_string());
}

/// What a code's text is given: the partition's name, then `detail`.
pub fn named(name: Option<&str>, detail: &str) -> String {
    match (name, detail.is_empty()) {
        (None, _) => detail.to_string(),
        (Some(name), true) => name.to_string(),
        (Some(name), false) => format!("{name}, {detail}"),
    }
}

/// Report a reference code on standard error, for the partition named.
pub fn post(code: &refcode::RefCode, detail: &str) {
    let detail = named(PARTITION.get().map(String::as_str), detail);
    eprintln!(
        "{} {} {}",
        refcode::WORD,
        code.code,
        code.text_with(&detail)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_names_its_partition_before_what_it_adds() {
        assert_eq!(named(Some("PROD"), ""), "PROD");
        assert_eq!(named(Some("PROD"), "side A"), "PROD, side A");
        assert_eq!(named(None, "the configuration"), "the configuration");
        assert_eq!(
            log_dir(Path::new("/data"), 7),
            PathBuf::from("/data/log/partitions/7")
        );
        assert_eq!(
            socket_dir(Path::new("/run"), 7).join(CONSOLE_SOCKET),
            PathBuf::from("/run/partitions/7/console.sock")
        );
    }
}
