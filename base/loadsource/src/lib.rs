// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The disks an appliance starts from. A load-source disk is a GPT
//! with the ESP (both slots' UKIs), the record partition and the data
//! partition. There is one, or two when the load source is mirrored:
//! then each disk has its own ESP and record partition, and the data
//! file system is one xfs on a protected set over the two data
//! partitions. The partition types, the names and the records' magics
//! are the appliance's (`appliance::LoadSourceNames`), so no appliance
//! takes another's disk for its own.
//!
//! - [`layout`]: the partitions, their types and names.
//! - [`record`]: the A/B record, version 2, and the slot rule.
//! - [`set`]: the label of a mirrored load source.
//! - [`multi`]: one record kept on every load-source disk.
//! - [`gpt`], [`disks`]: finding the load-source disks of a running host.
//! - [`boot`]: the firmware's boot entries.
//! - [`prepare`]: writing a load-source disk.
//! - [`payload`]: what the installation media carries to write it from.
//!
//! `unsafe` lives only in `sys` (one ioctl).

#![deny(unsafe_code)]

pub mod boot;
#[cfg(target_os = "linux")]
pub mod disks;
pub mod gpt;
pub mod layout;
pub mod multi;
pub mod payload;
pub mod prepare;
pub mod record;
pub mod set;
#[cfg(target_os = "linux")]
pub mod sys;
