// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The two ioctls this crate makes, both on a block device a LUN is
//! kept in: give a range back, and make a range read as zeros.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;

use rustix::ioctl::{Opcode, Setter, ioctl, opcode};

/// `_IO(0x12, 119)`: `BLKDISCARD`.
const BLKDISCARD: Opcode = opcode::none(0x12, 119);
/// `_IO(0x12, 127)`: `BLKZEROOUT`.
const BLKZEROOUT: Opcode = opcode::none(0x12, 127);

/// Discard `bytes` from `offset` of a block device.
pub fn discard(device: &File, offset: u64, bytes: u64) -> io::Result<()> {
    // SAFETY: BLKDISCARD reads one `uint64_t[2]`, the start and the
    // length, from the pointer it is given and writes nothing to user
    // memory; `Setter` passes a pointer to exactly that.
    unsafe { ioctl(device, Setter::<BLKDISCARD, [u64; 2]>::new([offset, bytes])) }?;
    Ok(())
}

/// Make `bytes` from `offset` of a block device read as zeros.
pub fn zero_out(device: &File, offset: u64, bytes: u64) -> io::Result<()> {
    // SAFETY: BLKZEROOUT reads one `uint64_t[2]`, as BLKDISCARD does,
    // and writes nothing to user memory.
    unsafe { ioctl(device, Setter::<BLKZEROOUT, [u64; 2]>::new([offset, bytes])) }?;
    Ok(())
}
