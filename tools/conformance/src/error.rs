// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Harness errors. A failed step carries the serial log so far.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Why the suite could not run or did not pass.
#[derive(Debug, Error)]
pub enum HarnessError {
    /// The image the suite was asked to boot is missing.
    #[error("{what} not found; run `all` first")]
    ImageMissing {
        /// Which image.
        what: &'static str,
    },
    /// QEMU is not installed or not on `PATH`.
    #[error("{binary} not found on PATH (or set {env})")]
    QemuNotFound {
        /// Binary name.
        binary: &'static str,
        /// Override variable.
        env: &'static str,
    },
    /// No OVMF image was found.
    #[error("OVMF {what} not found; set {env} or install it ({hint}); tried {tried:?}")]
    FirmwareNotFound {
        /// The code image or the variable store template.
        what: &'static str,
        /// Variable that names the file.
        env: &'static str,
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
    /// A step could not be set up.
    #[error("step {step}: {reason}")]
    Setup {
        /// Step name.
        step: &'static str,
        /// What went wrong.
        reason: String,
    },
    /// A step did not pass.
    #[error("step {step}: {reason}")]
    Failed {
        /// Step name.
        step: &'static str,
        /// What went wrong.
        reason: String,
        /// Serial log of the session.
        log: String,
    },
    /// Any other I/O problem.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl HarnessError {
    /// The captured serial log, when a step failed.
    pub fn log(&self) -> Option<&str> {
        match self {
            HarnessError::Failed { log, .. } => Some(log),
            _ => None,
        }
    }
}
