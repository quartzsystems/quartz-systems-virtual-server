// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 11's suites of the host's boundaries (`docs/SECURITY.md`).
//!
//! - [`vmm`]: a VMM confined: what a running one can see and call, and
//!   that one which tries a forbidden call ends alone.
//! - [`panic`]: a host whose kernel panics: restarted, the panic kept,
//!   posted and in the next host dump.
//! - [`memory`]: a partition's memory, given to the next, holds nothing
//!   of the one before.
//! - [`secureboot`]: a release's signed UKIs under Secure Boot, and an
//!   image that is not signed for it refused.

pub mod memory;
pub mod panic;
pub mod secureboot;
pub mod vmm;

use super::base::SuiteFn;

/// The suites.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("vmm-confined", vmm::confined),
    ("host-panic", panic::panicked),
    ("secure-boot", secureboot::secure_boot),
    ("memory-clean", memory::clean),
];
