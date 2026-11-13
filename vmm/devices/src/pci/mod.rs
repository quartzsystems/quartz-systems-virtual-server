// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The PCI bus of a partition: one bus, a host bridge at device 0 and
//! a virtio device at each device number after it, function 0 only.
//!
//! - [`config`]: a function's configuration space: header, memory
//!   BARs, capabilities.
//! - [`bus`]: the host bridge: configuration access by the two ports
//!   and by memory, and the BARs' addresses.
//! - [`msix`]: the MSI-X table of a function.

pub mod bus;
pub mod config;
pub mod msix;

pub use bus::{PciBus, PciDevice};
pub use config::Config;
pub use msix::Msix;
