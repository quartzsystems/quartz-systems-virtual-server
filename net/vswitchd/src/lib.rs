// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The switch service: the host's virtual switches as VLAN-filtering
//! bridges, their uplinks, the partitions' ports, over a local socket
//! (`docs/NETWORK.md`, `docs/VSWITCHD.md`). It is separate from the
//! network service, so a fault in switch code cannot take the
//! management network away.
//!
//! - [`proto`]: its socket, which QSI's virtual Ethernet maps onto.
//! - [`manager`]: the service itself, over a [`kernel::Kernel`].
//! - [`kernel`]: what it asks of the kernel.
//! - [`serve`]: the service on Linux.

#![forbid(unsafe_code)]

pub mod kernel;
pub mod manager;
pub mod proto;
#[cfg(target_os = "linux")]
pub mod serve;

pub use proto::*;
