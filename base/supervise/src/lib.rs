// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Supervision for an appliance's PID 1.
//!
//! - [`table`]: the fixed service table and the restart policy.
//! - [`control`]: the protocol of PID 1's control socket, framed with
//!   QS/400's `ipc`: status, power off, restart, and what services tell
//!   PID 1.
//! - [`run`]: the supervisor itself: it starts the table in order,
//!   takes each service's readiness byte and its output, restarts what
//!   exits and stops everything in reverse (Linux).
//! - [`ready`]: what a service calls to say it is ready.

#![forbid(unsafe_code)]

pub mod control;
pub mod ready;
#[cfg(target_os = "linux")]
pub mod run;
pub mod table;

pub use table::{Policy, Service, Verdict};
