// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Harness errors. A failed step carries the serial log so far.

use std::io;

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
    /// QEMU or OVMF is missing, or QEMU could not be started.
    #[error(transparent)]
    Qemu(#[from] driver::Error),
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
