// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A bus of address ranges, each a device's. The I/O ports are one.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// A device on a bus. `offset` is from the start of its range.
pub trait BusDevice: Send {
    /// Read `data.len()` bytes at `offset`.
    fn read(&mut self, offset: u64, data: &mut [u8]);
    /// Write `data` at `offset`.
    fn write(&mut self, offset: u64, data: &[u8]);
}

/// A device shared between the bus and whoever else drives it.
pub type Shared = Arc<Mutex<dyn BusDevice>>;

/// Ranges of addresses and the device at each.
#[derive(Default)]
pub struct Bus {
    ranges: BTreeMap<u64, (u64, Shared)>,
}

impl Bus {
    /// An empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Put `device` at `start` for `len` addresses. False when the
    /// range is empty or overlaps another.
    pub fn insert(&mut self, start: u64, len: u64, device: Shared) -> bool {
        let Some(end) = start.checked_add(len) else {
            return false;
        };
        if len == 0
            || self
                .ranges
                .iter()
                .any(|(s, (l, _))| *s < end && start < s + l)
        {
            return false;
        }
        self.ranges.insert(start, (len, device));
        true
    }

    fn find(&self, address: u64) -> Option<(u64, &Shared)> {
        let (start, (len, device)) = self.ranges.range(..=address).next_back()?;
        (address - start < *len).then_some((address - start, device))
    }

    /// Read from the device at `address`. False, and all ones in
    /// `data`, when nothing is there.
    pub fn read(&self, address: u64, data: &mut [u8]) -> bool {
        if let Some((offset, device)) = self.find(address) {
            if let Ok(mut device) = device.lock() {
                device.read(offset, data);
            }
            true
        } else {
            data.fill(0xFF);
            false
        }
    }

    /// Write to the device at `address`. False when nothing is there.
    pub fn write(&self, address: u64, data: &[u8]) -> bool {
        match self.find(address) {
            Some((offset, device)) => {
                if let Ok(mut device) = device.lock() {
                    device.write(offset, data);
                }
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Latch(Vec<(u64, u8)>);

    impl BusDevice for Latch {
        fn read(&mut self, offset: u64, data: &mut [u8]) {
            data.fill(u8::try_from(offset).unwrap());
        }
        fn write(&mut self, offset: u64, data: &[u8]) {
            self.0.push((offset, data[0]));
        }
    }

    #[test]
    fn an_address_reaches_the_device_of_its_range() {
        let latch = Arc::new(Mutex::new(Latch(Vec::new())));
        let mut bus = Bus::new();
        assert!(bus.insert(0x3F8, 8, latch.clone()));
        assert!(!bus.insert(0x3FF, 2, latch.clone()));
        assert!(!bus.insert(0x3F0, 9, latch.clone()));
        assert!(!bus.insert(u64::MAX, 2, latch.clone()));
        assert!(bus.insert(0x400, 1, latch.clone()));

        let mut byte = [0u8];
        assert!(bus.read(0x3FD, &mut byte));
        assert_eq!(byte, [5]);
        assert!(!bus.read(0x3F7, &mut byte));
        assert_eq!(byte, [0xFF]);
        assert!(bus.write(0x3F9, &[7]));
        assert!(!bus.write(0x401, &[7]));
        assert_eq!(latch.lock().unwrap().0, [(1, 7)]);
    }
}
