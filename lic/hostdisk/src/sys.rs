// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The one ioctl this crate makes: the kernel reads a disk's partition
//! table again after it was written.

#![allow(unsafe_code)]

use std::fs::File;

use rustix::ioctl::{NoArg, Opcode, ioctl, opcode};

/// `_IO(0x12, 95)`: `BLKRRPART`.
const BLKRRPART: Opcode = opcode::none(0x12, 95);

/// Have the kernel read the partition table of `disk` again.
pub fn reread_partitions(disk: &File) -> Result<(), String> {
    // SAFETY: BLKRRPART takes no argument and writes nothing to user
    // memory; `disk` is an open descriptor for the call's duration.
    unsafe { ioctl(disk, NoArg::<BLKRRPART>::new()) }.map_err(|e| e.to_string())
}
