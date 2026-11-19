// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The QEMU driver every appliance's conformance harness runs its
//! images under: a `q35` machine with OVMF, the serial port captured
//! and waited on, the monitor for keys and power events, extra disks,
//! network interfaces and removable volumes, and a kill for a power
//! pull. What is a product's own (its variables' names, its test
//! hooks, its screens) the harness passes in or keeps.
//!
//! - [`launch`]: what to run, and the QEMU command line for it.
//! - [`firmware`]: OVMF's code image and variable store template.
//! - [`vars`]: a copy of the variable store with boot entries in it.
//! - [`session`]: one running machine.
//! - [`serial`], [`term`]: the serial port as text and as a screen.
//! - [`wire`]: a station on the far side of a machine's NICs.
//! - [`disk`]: disk images made and copied.

pub mod disk;
pub mod firmware;
pub mod launch;
pub mod serial;
pub mod session;
pub mod term;
pub mod vars;
pub mod wire;

use std::io;
use std::path::PathBuf;

pub use launch::{Drive, Far, Launch, Passed, UplinkNic, args};
pub use session::Session;

/// Why a machine could not be launched.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// QEMU is not installed or not on `PATH`.
    #[error("{binary} not found on PATH (or set {env})")]
    QemuNotFound {
        /// Binary name.
        binary: &'static str,
        /// Override variable.
        env: String,
    },
    /// No OVMF image was found.
    #[error("OVMF {what} not found; set {env} or install it ({hint}); tried {tried:?}")]
    FirmwareNotFound {
        /// The code image or the variable store template.
        what: &'static str,
        /// Variable that names the file.
        env: String,
        /// Paths that were tried.
        tried: Vec<PathBuf>,
        /// Package hint.
        hint: &'static str,
    },
    /// QEMU could not be started.
    #[error("cannot start {qemu}: {source}")]
    Spawn {
        /// Binary that was launched.
        qemu: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
}
