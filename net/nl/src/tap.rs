// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Tap devices, through `/dev/net/tun`: an Ethernet tap with no packet
//! information and with virtio-net headers, as vhost-net wants it. A
//! tap is not persistent: it goes when the last open file of it is
//! closed, so a partition that dies takes its tap, and the tap's bridge
//! port, with it.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;

/// `IFF_TAP`.
const IFF_TAP: u16 = 0x0002;
/// `IFF_NO_PI`.
const IFF_NO_PI: u16 = 0x1000;
/// `IFF_VNET_HDR`.
const IFF_VNET_HDR: u16 = 0x4000;
/// `TUNSETIFF`: `_IOW('T', 202, int)`.
const TUNSETIFF: libc::Ioctl = 0x4004_54CA;
/// `TUNSETVNETHDRSZ`: `_IOW('T', 216, int)`.
const TUNSETVNETHDRSZ: libc::Ioctl = 0x4004_54D8;
/// `TUNSETOFFLOAD`: `_IOW('T', 208, unsigned int)`.
const TUNSETOFFLOAD: libc::Ioctl = 0x4004_54D0;

/// `TUN_F_CSUM`: the guest's frames may leave their checksum to us.
pub const TUN_F_CSUM: u32 = 0x01;
/// `TUN_F_TSO4`.
pub const TUN_F_TSO4: u32 = 0x02;
/// `TUN_F_TSO6`.
pub const TUN_F_TSO6: u32 = 0x04;
/// `TUN_F_TSO_ECN`.
pub const TUN_F_TSO_ECN: u32 = 0x08;

/// The virtio-net header with mergeable buffers and of version 1: what
/// a tap is told its frames carry.
pub const VNET_HDR_LEN: i32 = 12;

/// The longest interface name, without its NUL.
pub const NAME_MAX: usize = 15;

/// `struct ifreq` as `TUNSETIFF` takes it: the name, the flags, and the
/// rest of the union.
#[repr(C)]
struct IfReq {
    name: [u8; 16],
    flags: u16,
    _pad: [u8; 22],
}

/// Make the tap `name` and return its open file: the only one there is
/// of it.
pub fn create(name: &str) -> io::Result<File> {
    if name.is_empty() || name.len() > NAME_MAX || name.contains(['/', '\0', ' ']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("tap name {name:?}"),
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/net/tun")?;
    let mut req = IfReq {
        name: [0; 16],
        flags: IFF_TAP | IFF_NO_PI | IFF_VNET_HDR,
        _pad: [0; 22],
    };
    req.name[..name.len()].copy_from_slice(name.as_bytes());
    // SAFETY: TUNSETIFF reads and writes an ifreq, which `req` is, for
    // the length of the call.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), TUNSETIFF, &raw mut req) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    let size: libc::c_int = VNET_HDR_LEN;
    // SAFETY: TUNSETVNETHDRSZ reads an int.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), TUNSETVNETHDRSZ, &raw const size) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

/// Tell the tap of `file` which offloads the guest takes: `TUN_F_*`.
pub fn set_offload(file: &File, flags: u32) -> io::Result<()> {
    // SAFETY: TUNSETOFFLOAD takes its argument by value.
    let r = unsafe { libc::ioctl(file.as_raw_fd(), TUNSETOFFLOAD, libc::c_ulong::from(flags)) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
