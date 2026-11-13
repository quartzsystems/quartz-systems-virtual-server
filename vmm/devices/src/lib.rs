// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The devices of a partition.
//!
//! - [`bus`]: the I/O port bus the legacy devices are on.
//! - [`platform`]: what a device asks of the hypervisor: an interrupt
//!   line, a message interrupt's route, a notification by eventfd.
//! - [`legacy`]: the 16550, the CMOS clock, the keyboard controller's
//!   reset line, the ACPI sleep register.
//! - [`pci`]: the host bridge, configuration space, MSI-X.
//! - [`virtio`]: the virtio-pci transport (modern, version 1),
//!   virtio-scsi and virtio-rng.
//!
//! Every byte a guest controls is taken for hostile: a register write,
//! a descriptor, a SCSI command. Nothing here panics on one, reads or
//! writes outside what the guest was given, or allocates by a length
//! the guest names (`docs/VMM.md`).
//!
//! `unsafe` lives only in `sys` (two block-device ioctls).

#![deny(unsafe_code)]

pub mod bus;
pub mod legacy;
pub mod pci;
pub mod platform;
#[cfg(target_os = "linux")]
mod sys;
pub mod virtio;

/// The guest's memory.
pub type Memory = vm_memory::GuestMemoryMmap<()>;
