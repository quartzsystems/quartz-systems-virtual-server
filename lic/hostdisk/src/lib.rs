// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The disk a QS/VS host boots from: a GPT with the ESP (both slots'
//! UKIs), the A/B record partition and `QSVSDATA` (xfs). Storage pools
//! are on other disks. The build writes this as an image file; the
//! installer of a later phase writes the same description to a disk.

#![forbid(unsafe_code)]

pub mod layout;
#[cfg(target_os = "linux")]
pub mod locate;
pub mod record;
