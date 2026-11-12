// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The disks a QS/VS host boots from. A load-source disk is a GPT with
//! the ESP (both slots' UKIs), the record partition and `QSVSDATA`.
//! There is one, or two when the load source is mirrored: then each
//! disk has its own ESP and record partition, and `QSVSDATA` is one
//! xfs on a protected set over the two data partitions. Storage pools
//! are on other disks.
//!
//! - [`layout`]: the partitions, their types and names.
//! - [`record`]: the A/B record, version 2, and the slot rule.
//! - [`set`]: the label of a mirrored load source.
//! - [`multi`]: one record kept on every load-source disk.
//! - [`gpt`], [`disks`]: finding the load-source disks of a running host.
//! - [`boot`]: the firmware's boot entries.
//! - [`prepare`]: writing a load-source disk.
//!
//! `unsafe` lives only in `sys` (one ioctl).

#![deny(unsafe_code)]

pub mod boot;
#[cfg(target_os = "linux")]
pub mod disks;
pub mod gpt;
pub mod layout;
pub mod multi;
pub mod prepare;
pub mod record;
pub mod set;
#[cfg(target_os = "linux")]
pub mod sys;
