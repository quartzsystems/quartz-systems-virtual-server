// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS conformance harness. Boots the disk images or the
//! installation ISO under `qemu-system-x86_64` with OVMF, captures the
//! serial console and checks what PID 1 says; the base system's suites
//! also ask the control socket for its status, through the test hooks'
//! channel of a test image. `build test` drives it.

#![forbid(unsafe_code)]

pub mod error;
pub mod qemu;
pub mod suite;

pub use error::HarnessError;
pub use qemu::firmware;
pub use suite::{Media, Report, Suite};
