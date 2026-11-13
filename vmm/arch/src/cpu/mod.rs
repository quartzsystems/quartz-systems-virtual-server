// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a partition's processors are told about themselves.
//!
//! - [`cpuid`]: what KVM supports, with the topology of the partition.
//! - [`msr`]: the model-specific registers a processor starts with.
//! - [`lapic`]: the local APIC's two local interrupt pins.

pub mod cpuid;
pub mod lapic;
pub mod msr;

/// The most processors a partition may have: APIC ids fit a byte in
/// the MADT entries written here.
pub const MAX_VCPUS: u32 = 240;
