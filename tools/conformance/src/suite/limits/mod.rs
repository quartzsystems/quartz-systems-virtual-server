// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 11's limits and baselines (`docs/PHASE11.md`).
//!
//! - [`reached`]: each of a host's limits reached, and one past it
//!   refused with its message.
//! - [`baseline`]: the numbers kept of a host: its boot, IPLs, pools,
//!   console and what idle partitions cost it. Run when named.

pub mod baseline;
pub mod reached;

use super::base::SuiteFn;

/// The suites.
pub const SUITES: &[(&str, SuiteFn)] = &[("limits", reached::limits)];

/// The suites run only when named: the baselines need a host of 8 GiB.
pub const NAMED: &[(&str, SuiteFn)] = &[("baselines", baseline::baselines)];
