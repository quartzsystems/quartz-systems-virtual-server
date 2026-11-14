// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Virtual switches as the host keeps them (`docs/NETWORK.md`).
//!
//! - [`switch`]: `vswitch.toml`: the switches, their VLANs, MTU and
//!   uplink, the limits, and the kernel's names for each.
//! - [`validate`]: every error, each a message.
//! - [`mac`]: a partition adapter's MAC address.
//! - [`port`]: what a partition's port is: its VLANs and its address.

#![forbid(unsafe_code)]

pub mod mac;
pub mod port;
pub mod switch;
pub mod validate;

pub use switch::{Bond, Limits, Switch, Switches, Uplink};
