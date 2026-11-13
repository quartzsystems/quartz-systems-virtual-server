// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The four devices a partition has that are not on the PCI bus.
//!
//! - [`serial`]: the 16550 the guest's console is on.
//! - [`cmos`]: the clock the guest reads its time from.
//! - [`reset`]: the keyboard controller's reset line, and nothing else
//!   of the keyboard controller.
//! - [`power`]: the ACPI sleep control register.

pub mod cmos;
pub mod power;
pub mod reset;
pub mod serial;
