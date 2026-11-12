// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Logging for an appliance.
//!
//! - [`record`]: one log record, as it crosses the socket.
//! - [`client`]: [`client::Sender`], which queues records until the
//!   log service takes them, so nothing said before it is up is lost.
//! - [`files`]: what is kept under `<data>/log/`: the NDJSON system
//!   log (rotated by size under a total cap), the reference code
//!   history (append-only) and the kernel log of each boot.
//! - [`serve`]: the service: a Unix datagram socket for records and
//!   `/dev/kmsg` (Linux).

#![forbid(unsafe_code)]

#[cfg(unix)]
pub mod client;
pub mod files;
pub mod record;
#[cfg(target_os = "linux")]
pub mod serve;

pub use record::{Record, Severity};
