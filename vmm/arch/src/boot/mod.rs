// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The state the 64-bit Linux boot protocol asks for: long mode with
//! paging on and the first four gigabytes mapped one to one, flat
//! segments, interrupts off, `rsi` at the zero page, and the jump to
//! the kernel's 64-bit entry, 0x200 past where it was loaded.
//!
//! - [`tables`]: the page tables and the GDT, written into guest memory.
//! - [`regs`]: the registers of the boot processor.
//! - [`zeropage`]: the boot parameters.
//! - [`reset`]: the reset vector, for a guest that restarts by it.

pub mod regs;
pub mod reset;
pub mod tables;
pub mod zeropage;

/// The 64-bit entry point is this far into the loaded kernel.
pub const ENTRY_OFFSET: u64 = 0x200;
