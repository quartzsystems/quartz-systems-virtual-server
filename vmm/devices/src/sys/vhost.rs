// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! vhost-net, on `/dev/vhost-net` (`linux/vhost.h`): the kernel's own
//! virtio-net data path over a tap. Each call passes a struct of the
//! layout the kernel reads, by pointer, for the length of the call.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

use rustix::ioctl::{Getter, IntegerSetter, NoArg, Opcode, Setter, Updater, ioctl, opcode};

/// The most memory regions a table here has.
pub const MAX_REGIONS: usize = 8;

/// `struct vhost_vring_state`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VringState {
    /// The queue.
    pub index: u32,
    /// Its size, or its base.
    pub num: u32,
}

/// `struct vhost_vring_file`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VringFile {
    /// The queue.
    pub index: u32,
    /// The descriptor; -1 for none.
    pub fd: i32,
}

/// `struct vhost_vring_addr`: the rings at the VMM's own addresses.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VringAddr {
    /// The queue.
    pub index: u32,
    /// No flag is used.
    pub flags: u32,
    /// The descriptor table.
    pub desc: u64,
    /// The used ring.
    pub used: u64,
    /// The available ring.
    pub avail: u64,
    /// No dirty log.
    pub log: u64,
}

/// `struct vhost_memory_region`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemRegion {
    /// Where it is in the guest.
    pub guest: u64,
    /// Its length.
    pub size: u64,
    /// Where it is in the VMM.
    pub user: u64,
    /// Zero.
    pub padding: u64,
}

/// `struct vhost_memory` with room for [`MAX_REGIONS`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MemTable {
    /// How many regions follow.
    pub count: u32,
    /// Zero.
    pub padding: u32,
    /// The regions.
    pub regions: [MemRegion; MAX_REGIONS],
}

const GET_FEATURES: Opcode = opcode::read::<u64>(0xAF, 0x00);
const SET_FEATURES: Opcode = opcode::write::<u64>(0xAF, 0x00);
const SET_OWNER: Opcode = opcode::none(0xAF, 0x01);
// The kernel's size is the header's; the regions follow it.
const SET_MEM_TABLE: Opcode = opcode::write::<[u32; 2]>(0xAF, 0x03);
const SET_VRING_NUM: Opcode = opcode::write::<VringState>(0xAF, 0x10);
const SET_VRING_ADDR: Opcode = opcode::write::<VringAddr>(0xAF, 0x11);
const SET_VRING_BASE: Opcode = opcode::write::<VringState>(0xAF, 0x12);
const GET_VRING_BASE: Opcode = opcode::read_write::<VringState>(0xAF, 0x12);
const SET_VRING_KICK: Opcode = opcode::write::<VringFile>(0xAF, 0x20);
const SET_VRING_CALL: Opcode = opcode::write::<VringFile>(0xAF, 0x21);
const NET_SET_BACKEND: Opcode = opcode::write::<VringFile>(0xAF, 0x30);
/// `TUNSETOFFLOAD`, on the tap: its argument by value.
const TUN_SET_OFFLOAD: Opcode = opcode::write::<u32>(b'T', 208);

/// The features vhost-net offers.
pub fn features(vhost: &File) -> io::Result<u64> {
    // SAFETY: VHOST_GET_FEATURES writes one u64 to the pointer it is
    // given; `Getter` gives it one.
    Ok(unsafe { ioctl(vhost, Getter::<GET_FEATURES, u64>::new()) }?)
}

/// Take the features the driver accepted that vhost-net serves.
pub fn set_features(vhost: &File, features: u64) -> io::Result<()> {
    // SAFETY: VHOST_SET_FEATURES reads one u64.
    Ok(unsafe { ioctl(vhost, Setter::<SET_FEATURES, u64>::new(features)) }?)
}

/// Make this process the device's owner.
pub fn set_owner(vhost: &File) -> io::Result<()> {
    // SAFETY: VHOST_SET_OWNER takes no argument.
    Ok(unsafe { ioctl(vhost, NoArg::<SET_OWNER>::new()) }?)
}

/// The guest's memory, as `table` says it.
pub fn set_mem_table(vhost: &File, table: &MemTable) -> io::Result<()> {
    if table.count as usize > MAX_REGIONS {
        return Err(io::Error::other("too many memory regions"));
    }
    // SAFETY: VHOST_SET_MEM_TABLE reads the header and then `count`
    // regions, which `table` has room for.
    Ok(unsafe { ioctl(vhost, Setter::<SET_MEM_TABLE, MemTable>::new(*table)) }?)
}

/// A queue's size.
pub fn set_vring_num(vhost: &File, index: u32, num: u32) -> io::Result<()> {
    let state = VringState { index, num };
    // SAFETY: reads one vhost_vring_state.
    Ok(unsafe { ioctl(vhost, Setter::<SET_VRING_NUM, VringState>::new(state)) }?)
}

/// A queue's rings.
pub fn set_vring_addr(vhost: &File, addr: VringAddr) -> io::Result<()> {
    // SAFETY: reads one vhost_vring_addr.
    Ok(unsafe { ioctl(vhost, Setter::<SET_VRING_ADDR, VringAddr>::new(addr)) }?)
}

/// Where a queue's available ring is to be read from.
pub fn set_vring_base(vhost: &File, index: u32, num: u32) -> io::Result<()> {
    let state = VringState { index, num };
    // SAFETY: reads one vhost_vring_state.
    Ok(unsafe { ioctl(vhost, Setter::<SET_VRING_BASE, VringState>::new(state)) }?)
}

/// Stop a queue: where its available ring was read to.
pub fn get_vring_base(vhost: &File, index: u32) -> io::Result<u32> {
    let mut state = VringState { index, num: 0 };
    // SAFETY: reads and writes one vhost_vring_state, `state`.
    unsafe {
        ioctl(
            vhost,
            Updater::<GET_VRING_BASE, VringState>::new(&mut state),
        )
    }?;
    Ok(state.num)
}

fn raw(fd: Option<&impl AsRawFd>) -> i32 {
    fd.map_or(-1, AsRawFd::as_raw_fd)
}

/// The eventfd the guest's notifications of a queue are written to.
pub fn set_vring_kick(vhost: &File, index: u32, fd: Option<&impl AsRawFd>) -> io::Result<()> {
    let file = VringFile { index, fd: raw(fd) };
    // SAFETY: reads one vhost_vring_file.
    Ok(unsafe { ioctl(vhost, Setter::<SET_VRING_KICK, VringFile>::new(file)) }?)
}

/// The eventfd that raises a queue's interrupt in the guest.
pub fn set_vring_call(vhost: &File, index: u32, fd: Option<&impl AsRawFd>) -> io::Result<()> {
    let file = VringFile { index, fd: raw(fd) };
    // SAFETY: reads one vhost_vring_file.
    Ok(unsafe { ioctl(vhost, Setter::<SET_VRING_CALL, VringFile>::new(file)) }?)
}

/// The tap a queue is served from; none stops it.
pub fn set_backend(vhost: &File, index: u32, tap: Option<&impl AsRawFd>) -> io::Result<()> {
    let file = VringFile {
        index,
        fd: raw(tap),
    };
    // SAFETY: reads one vhost_vring_file.
    Ok(unsafe { ioctl(vhost, Setter::<NET_SET_BACKEND, VringFile>::new(file)) }?)
}

/// Tell a tap which offloads the guest takes (`TUN_F_*`).
pub fn set_offload(tap: &File, flags: u32) -> io::Result<()> {
    // SAFETY: TUNSETOFFLOAD takes its argument by value and reads no
    // memory.
    Ok(unsafe {
        ioctl(
            tap,
            IntegerSetter::<TUN_SET_OFFLOAD>::new_usize(flags as usize),
        )
    }?)
}
