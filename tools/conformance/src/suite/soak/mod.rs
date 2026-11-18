// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The long runs of Phase 11, named one at a time and never part of
//! `test`'s whole run: each takes hours.
//!
//! - [`faults`]: Section 1's soak. Partitions started over and over under
//!   load, their kernels' faults counted per thousand IPLs and per
//!   hour, each faulted partition held, dumped and its dump read.
//! - [`day`]: Section 8's: a day of a host worked by a script under
//!   load, and nothing left behind.
//! - [`report`]: what a soak found, as text and as JSON lines.
//! - [`core`]: a dump's printable records found, for a first reading.

pub mod core;
pub mod day;
pub mod faults;
pub mod report;

use std::time::Duration;

use super::base::SuiteFn;

/// The long suites.
pub const SUITES: &[(&str, SuiteFn)] = &[("soak-faults", faults::faults), ("soak-day", day::day)];

/// A number from the environment, or `default`.
pub fn knob(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

/// Words from the environment, or none.
pub fn words(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// `QSVS_SOAK_MINUTES`: how long a soak runs.
pub fn length() -> Duration {
    Duration::from_secs(knob("QSVS_SOAK_MINUTES", 60) * 60)
}
