// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A function's configuration space: 256 bytes, each with a mask of
//! the bits the guest may change. A write changes those and nothing
//! else, whatever its offset and length.

/// Bytes of a configuration space.
pub const CONFIG_BYTES: usize = 256;

const VENDOR: usize = 0x00;
const DEVICE: usize = 0x02;
const COMMAND: usize = 0x04;
const STATUS: usize = 0x06;
const REVISION: usize = 0x08;
const CLASS: usize = 0x09;
const BAR0: usize = 0x10;
const SUBSYSTEM_VENDOR: usize = 0x2C;
const SUBSYSTEM: usize = 0x2E;
const CAPABILITIES: usize = 0x34;
/// Capabilities start after the header.
const FIRST_CAPABILITY: usize = 0x40;

/// Command: answer memory accesses.
pub const COMMAND_MEMORY: u16 = 1 << 1;
/// Command bits a guest may set: I/O, memory, bus master, `INTx` off.
const COMMAND_WRITABLE: u16 = 0x0407;
/// Status: there is a capability list.
const STATUS_CAPABILITIES: u16 = 1 << 4;

/// BARs a function may have here: 32-bit memory ones.
pub const BARS: usize = 6;

/// A function's identity.
#[derive(Clone, Copy, Debug)]
pub struct Identity {
    /// The vendor.
    pub vendor: u16,
    /// The device.
    pub device: u16,
    /// The revision.
    pub revision: u8,
    /// Class, subclass and programming interface, as one number.
    pub class: u32,
    /// The subsystem vendor.
    pub subsystem_vendor: u16,
    /// The subsystem.
    pub subsystem: u16,
}

/// The configuration space of one function.
pub struct Config {
    bytes: [u8; CONFIG_BYTES],
    writable: [u8; CONFIG_BYTES],
    bar_bytes: [u32; BARS],
    next_capability: usize,
    last_capability: Option<usize>,
}

impl Config {
    /// The space of a function with `identity`.
    pub fn new(identity: &Identity) -> Self {
        let mut c = Self {
            bytes: [0; CONFIG_BYTES],
            writable: [0; CONFIG_BYTES],
            bar_bytes: [0; BARS],
            next_capability: FIRST_CAPABILITY,
            last_capability: None,
        };
        c.set16(VENDOR, identity.vendor);
        c.set16(DEVICE, identity.device);
        c.bytes[REVISION] = identity.revision;
        let class = identity.class.to_le_bytes();
        c.bytes[CLASS..CLASS + 3].copy_from_slice(&class[..3]);
        c.set16(SUBSYSTEM_VENDOR, identity.subsystem_vendor);
        c.set16(SUBSYSTEM, identity.subsystem);
        c.writable[COMMAND..COMMAND + 2].copy_from_slice(&COMMAND_WRITABLE.to_le_bytes());
        c
    }

    fn set16(&mut self, at: usize, value: u16) {
        self.bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn get16(&self, at: usize) -> u16 {
        u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]])
    }

    fn get32(&self, at: usize) -> u32 {
        u32::from_le_bytes([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ])
    }

    /// Give the function a 32-bit memory BAR of `bytes` (a power of
    /// two, 4096 at least) at `address`. False when it cannot be.
    pub fn add_bar(&mut self, index: usize, address: u32, bytes: u32) -> bool {
        if index >= BARS
            || self.bar_bytes[index] != 0
            || !bytes.is_power_of_two()
            || bytes < 4096
            || !address.is_multiple_of(bytes)
        {
            return false;
        }
        let at = BAR0 + index * 4;
        self.bytes[at..at + 4].copy_from_slice(&address.to_le_bytes());
        // The guest sizes a BAR by writing ones: only the address bits take.
        self.writable[at..at + 4].copy_from_slice(&(!(bytes - 1)).to_le_bytes());
        self.bar_bytes[index] = bytes;
        true
    }

    /// Where BAR `index` is now, when the function answers memory
    /// accesses and has that BAR.
    pub fn bar(&self, index: usize) -> Option<(u64, u64)> {
        let bytes = *self.bar_bytes.get(index)?;
        if bytes == 0 || self.get16(COMMAND) & COMMAND_MEMORY == 0 {
            return None;
        }
        let address = self.get32(BAR0 + index * 4) & !(bytes - 1);
        Some((u64::from(address), u64::from(bytes)))
    }

    /// The BAR and the offset in it of `address`.
    pub fn bar_at(&self, address: u64) -> Option<(usize, u64)> {
        (0..BARS).find_map(|index| {
            let (start, bytes) = self.bar(index)?;
            let offset = address.checked_sub(start)?;
            (offset < bytes).then_some((index, offset))
        })
    }

    /// Add a capability: `id`, then the link, then `body`, whose
    /// writable bits are `writable`. Where it is; `None` when the
    /// space is full.
    pub fn add_capability(&mut self, id: u8, body: &[u8], writable: &[u8]) -> Option<usize> {
        let at = self.next_capability;
        let end = at.checked_add(2 + body.len())?;
        if end > CONFIG_BYTES || writable.len() > body.len() {
            return None;
        }
        self.bytes[at] = id;
        self.bytes[at + 2..end].copy_from_slice(body);
        self.writable[at + 2..at + 2 + writable.len()].copy_from_slice(writable);
        if let Some(last) = self.last_capability {
            self.bytes[last + 1] = u8::try_from(at).ok()?;
        } else {
            self.bytes[CAPABILITIES] = u8::try_from(at).ok()?;
            let status = self.get16(STATUS) | STATUS_CAPABILITIES;
            self.set16(STATUS, status);
        }
        self.last_capability = Some(at);
        // Capabilities are aligned to four bytes.
        self.next_capability = end.next_multiple_of(4);
        Some(at)
    }

    /// Read `data.len()` bytes at `offset`; ones beyond the space.
    pub fn read(&self, offset: usize, data: &mut [u8]) {
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = offset
                .checked_add(i)
                .and_then(|at| self.bytes.get(at))
                .copied()
                .unwrap_or(0xFF);
        }
    }

    /// Write `data` at `offset`: the writable bits change.
    pub fn write(&mut self, offset: usize, data: &[u8]) {
        for (i, value) in data.iter().enumerate() {
            let Some(at) = offset.checked_add(i).filter(|at| *at < CONFIG_BYTES) else {
                return;
            };
            let mask = self.writable[at];
            self.bytes[at] = (self.bytes[at] & !mask) | (value & mask);
        }
    }

    /// One byte, for a device that keeps state in a capability.
    pub fn byte(&self, at: usize) -> u8 {
        self.bytes.get(at).copied().unwrap_or(0xFF)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function() -> Config {
        Config::new(&Identity {
            vendor: 0x1AF4,
            device: 0x1048,
            revision: 1,
            class: 0x01_00_00,
            subsystem_vendor: 0x1AF4,
            subsystem: 8,
        })
    }

    fn read32(c: &Config, at: usize) -> u32 {
        let mut b = [0u8; 4];
        c.read(at, &mut b);
        u32::from_le_bytes(b)
    }

    #[test]
    fn the_header_identifies_the_function_and_is_read_only() {
        let mut c = function();
        assert_eq!(read32(&c, 0), 0x1048_1AF4);
        assert_eq!(read32(&c, 8), 0x0100_0001);
        c.write(0, &[0xFF; 4]);
        c.write(8, &[0xFF; 8]);
        assert_eq!(read32(&c, 0), 0x1048_1AF4);
        assert_eq!(read32(&c, 8), 0x0100_0001);
    }

    #[test]
    fn a_bar_is_sized_by_writing_ones_and_moves_when_written() {
        let mut c = function();
        assert!(c.add_bar(0, 0xC000_0000, 0x4000));
        assert!(!c.add_bar(0, 0xC000_0000, 0x4000));
        assert!(!c.add_bar(1, 0xC000_0000, 0x3000));
        assert!(!c.add_bar(1, 0xC000_1000, 0x4000));
        assert!(!c.add_bar(6, 0xC000_0000, 0x4000));
        // Memory decoding is off until the guest turns it on.
        assert_eq!(c.bar(0), None);
        c.write(COMMAND, &COMMAND_MEMORY.to_le_bytes());
        assert_eq!(c.bar(0), Some((0xC000_0000, 0x4000)));
        assert_eq!(c.bar_at(0xC000_3FFF), Some((0, 0x3FFF)));
        assert_eq!(c.bar_at(0xC000_4000), None);
        // An address below the BAR is not in it.
        assert_eq!(c.bar_at(0xBFFF_FFFF), None);
        assert_eq!(c.bar_at(0), None);

        c.write(BAR0, &[0xFF; 4]);
        assert_eq!(read32(&c, BAR0), 0xFFFF_C000);
        c.write(BAR0, &0xD000_0123u32.to_le_bytes());
        assert_eq!(c.bar(0), Some((0xD000_0000, 0x4000)));
    }

    #[test]
    fn capabilities_are_chained_and_only_their_writable_bits_change() {
        let mut c = function();
        let first = c.add_capability(0x09, &[0xAA; 14], &[]).unwrap();
        let second = c
            .add_capability(0x11, &[0x02, 0x00, 0, 0], &[0x00, 0xC0])
            .unwrap();
        assert_eq!(c.byte(CAPABILITIES), 0x40);
        assert_eq!(first, 0x40);
        assert_eq!(c.byte(first + 1), 0x50);
        assert_eq!(second, 0x50);
        assert_eq!(c.byte(second + 1), 0);
        assert_ne!(read32(&c, 4) >> 16 & 0x10, 0);
        c.write(second, &[0xFF; 6]);
        assert_eq!(c.byte(second), 0x11);
        assert_eq!(c.byte(second + 3), 0xC0);
        assert_eq!(c.byte(second + 2), 0x02);
        // The space does not overflow.
        while c.add_capability(0x09, &[0; 30], &[]).is_some() {}
        assert!(c.add_capability(0x09, &[0; 30], &[]).is_none());
    }

    #[test]
    fn accesses_beyond_the_space_read_ones_and_write_nothing() {
        let mut c = function();
        let mut data = [0u8; 4];
        c.read(254, &mut data);
        assert_eq!(data[2..], [0xFF, 0xFF]);
        c.read(usize::MAX, &mut data);
        assert_eq!(data, [0xFF; 4]);
        c.write(usize::MAX - 1, &[1, 2, 3, 4]);
        c.write(255, &[1, 2, 3, 4]);
    }
}
