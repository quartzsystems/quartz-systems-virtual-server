// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 10's suites: physical I/O and tape pass-through, on the test
//! image.
//!
//! - [`tape`]: Section 1, QS/400 saving to and restoring from the
//!   virtual tape drive.
//! - [`devices`]: a host's devices given to partitions.

pub mod devices;
mod tape;

use super::base::SuiteFn;

/// The suites, in the order of Phase 10's handoff: its first, every
/// earlier suite on the new pin, is the build tool's.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("pio-tape-save", tape::save),
    ("pio-tape-span", tape::span),
    ("pio-tape-volumes", tape::volumes),
    ("pio-nvme", devices::nvme),
];
