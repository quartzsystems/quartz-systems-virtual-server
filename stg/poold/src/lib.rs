// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The pool service: the host's storage pools, their virtual disks and
//! the media library (`docs/STORAGE.md`, `docs/POOLD.md`). It is
//! separate from the load source's storage service, so a fault in pool
//! code cannot take `/data` away.
//!
//! - [`proto`]: its socket, which the QSI family of Phase 8 maps onto.
//! - [`serve`]: the service itself (Linux).

#![forbid(unsafe_code)]

pub mod proto;
#[cfg(target_os = "linux")]
pub mod serve;

pub use proto::*;
