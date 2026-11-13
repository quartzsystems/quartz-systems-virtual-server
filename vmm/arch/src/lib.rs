// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's `x86_64` boot state. A partition has no firmware: the
//! VMM puts the processor where the 64-bit Linux boot protocol wants
//! it and describes the machine in the zero page and in ACPI.
//!
//! - [`layout`]: the guest's physical memory map.
//! - [`boot`]: page tables, the GDT, the registers and the zero page.
//! - [`cpu`]: the CPUID a partition sees, its boot MSRs, its local APIC.
//!
//! Nothing here calls KVM: the structures are filled and the VMM hands
//! them over.

#![forbid(unsafe_code)]

pub mod boot;
pub mod cpu;
pub mod layout;

/// What can go wrong setting a partition's boot state.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The memory asked for is not one a partition can have.
    #[error("{0} bytes of memory: {1}")]
    Memory(u64, &'static str),
    /// A boot structure does not fit in guest memory.
    #[error("guest memory does not hold {0}")]
    GuestMemory(&'static str),
    /// More processors than the CPUID topology here describes.
    #[error("{0} processors: 1 to {max} are supported", max = cpu::MAX_VCPUS)]
    Processors(u32),
}
