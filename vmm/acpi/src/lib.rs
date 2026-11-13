// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The ACPI tables of a partition. There is no firmware to write them,
//! so the VMM does, as one block placed at [`arch::layout::ACPI_START`]
//! with the root pointer first.
//!
//! - [`tables`]: RSDP, XSDT, FADT (hardware-reduced), MADT, MCFG, and
//!   the block they are laid out in.
//! - [`dsdt`]: the devices that cannot be discovered: the PCI host
//!   bridge, the serial port, the clock, the power-off device.

#![forbid(unsafe_code)]

pub mod dsdt;
pub mod tables;

pub use tables::{Tables, build};

/// The OEM id of every table.
pub const OEM_ID: [u8; 6] = *b"QUARTZ";
/// The OEM table id of every table.
pub const OEM_TABLE_ID: [u8; 8] = *b"QSVSPART";
/// The OEM revision of every table.
pub const OEM_REVISION: u32 = 1;
