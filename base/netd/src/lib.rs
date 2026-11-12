// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The management network and the time of an appliance.
//!
//! - [`manage`]: the interface as a machine: the device (by name, by
//!   MAC, or the first with carrier), an optional VLAN, DHCP or a
//!   static address, carrier loss and return, reload. IPv4 only; IPv6
//!   is link-local.
//! - [`control`]: the service's socket: reload and status.
//! - [`time`]: the clock kept in step with the configured time
//!   servers.
//! - [`sys`] and [`serve`]: the machine on Linux, over QS/400's
//!   `netlink`, `dhcp` and `ntp`.
//!
//! Nothing here listens on the network.

#![deny(unsafe_code)]

pub mod control;
pub mod manage;
#[cfg(target_os = "linux")]
pub mod serve;
#[cfg(target_os = "linux")]
pub mod sys;
pub mod time;
