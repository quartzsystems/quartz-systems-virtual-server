// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager (`docs/PARTD.md`). It keeps every partition's
//! definition and profiles, gives out the host's processors and
//! memory at activation, and owns every `qsvmm` process: it starts
//! each in its partition's control group, watches it, and says what
//! state the partition is in. It is driven over a local socket, which
//! the QSI host server of Phase 8 will sit on.
//!
//! - [`proto`]: the socket: requests, answers, events.
//! - [`store`]: the partitions in state.
//! - [`manager`]: the partitions as they are, and what each request
//!   and each end of a VMM does to them.
//! - [`serve`]: the service: the machine behind the manager, the
//!   socket, the watchers, auto start (Linux).
//!
//! `unsafe` lives only in `serve::sys`.

#![deny(unsafe_code)]

pub mod manager;
pub mod proto;
pub mod store;

#[cfg(target_os = "linux")]
pub mod serve;

pub use proto::{Calls, Client, Event, Partitions, Posted, Request, State, Status};
