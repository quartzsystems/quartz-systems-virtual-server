// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host bridge and the one bus behind it. Configuration space is
//! reached two ways, both to the same registers: the address and data
//! ports, and memory at [`arch::layout::ECAM_START`]. A memory access
//! anywhere else in PCI memory space goes to the function whose BAR
//! is there.

use std::sync::{Arc, Mutex};

use arch::layout::{ECAM_BYTES, ECAM_START};

use crate::bus::BusDevice;
use crate::pci::config::{Config, Identity};

/// Device numbers of a bus.
pub const SLOTS: usize = 32;

/// The host bridge's identity: a generic PCI host bridge.
const BRIDGE: Identity = Identity {
    vendor: 0x1B36,
    device: 0x0008,
    revision: 0,
    class: 0x06_00_00,
    subsystem_vendor: 0,
    subsystem: 0,
};

/// A function on the bus.
pub trait PciDevice: Send {
    /// Read configuration space.
    fn read_config(&mut self, offset: usize, data: &mut [u8]);
    /// Write configuration space.
    fn write_config(&mut self, offset: usize, data: &[u8]);
    /// The BAR and offset of a memory address, when it is this
    /// function's.
    fn bar_at(&self, address: u64) -> Option<(usize, u64)>;
    /// Read a BAR.
    fn read_bar(&mut self, bar: usize, offset: u64, data: &mut [u8]);
    /// Write a BAR.
    fn write_bar(&mut self, bar: usize, offset: u64, data: &[u8]);
}

struct Bridge(Config);

impl PciDevice for Bridge {
    fn read_config(&mut self, offset: usize, data: &mut [u8]) {
        self.0.read(offset, data);
    }
    fn write_config(&mut self, offset: usize, data: &[u8]) {
        self.0.write(offset, data);
    }
    fn bar_at(&self, _: u64) -> Option<(usize, u64)> {
        None
    }
    fn read_bar(&mut self, _: usize, _: u64, data: &mut [u8]) {
        data.fill(0xFF);
    }
    fn write_bar(&mut self, _: usize, _: u64, _: &[u8]) {}
}

/// A function shared between the bus and its own threads.
pub type SharedPci = Arc<Mutex<dyn PciDevice>>;

/// The bus.
pub struct PciBus {
    slots: Vec<Option<SharedPci>>,
    /// What was last written to the configuration address port.
    address: u32,
}

impl PciBus {
    /// A bus with the host bridge at device 0.
    pub fn new() -> Self {
        let mut slots: Vec<Option<SharedPci>> = (0..SLOTS).map(|_| None).collect();
        slots[0] = Some(Arc::new(Mutex::new(Bridge(Config::new(&BRIDGE)))));
        Self { slots, address: 0 }
    }

    /// Put `device` at the next free device number; `None` when the
    /// bus is full.
    pub fn add(&mut self, device: SharedPci) -> Option<usize> {
        let slot = self.slots.iter().position(Option::is_none)?;
        self.slots[slot] = Some(device);
        Some(slot)
    }

    /// The function at bus, device, function; only bus 0 and function
    /// 0 have any.
    fn function(&self, bus: u32, device: u32, function: u32) -> Option<&SharedPci> {
        if bus != 0 || function != 0 {
            return None;
        }
        self.slots.get(usize::try_from(device).ok()?)?.as_ref()
    }

    fn read_config(&self, bus: u32, device: u32, function: u32, offset: usize, data: &mut [u8]) {
        match self.function(bus, device, function).map(|f| f.lock()) {
            Some(Ok(mut f)) => f.read_config(offset, data),
            _ => data.fill(0xFF),
        }
    }

    fn write_config(&self, bus: u32, device: u32, function: u32, offset: usize, data: &[u8]) {
        if let Some(Ok(mut f)) = self.function(bus, device, function).map(|f| f.lock()) {
            f.write_config(offset, data);
        }
    }

    /// Whether `address` is in memory-mapped configuration space.
    pub fn is_ecam(address: u64) -> bool {
        (ECAM_START..ECAM_START + ECAM_BYTES).contains(&address)
    }

    /// Bus, device, function and register of an offset into
    /// memory-mapped configuration space.
    #[allow(clippy::cast_possible_truncation)] // masked first
    fn ecam(offset: u64) -> (u32, u32, u32, usize) {
        (
            ((offset >> 20) & 0xFF) as u32,
            ((offset >> 15) & 0x1F) as u32,
            ((offset >> 12) & 0x7) as u32,
            (offset & 0xFFF) as usize,
        )
    }

    /// A memory read. False when nothing is at `address`: `data` is
    /// then all ones.
    pub fn read_memory(&self, address: u64, data: &mut [u8]) -> bool {
        if Self::is_ecam(address) {
            let (bus, device, function, offset) = Self::ecam(address - ECAM_START);
            self.read_config(bus, device, function, offset, data);
            return true;
        }
        for slot in self.slots.iter().flatten() {
            if let Ok(mut f) = slot.lock()
                && let Some((bar, offset)) = f.bar_at(address)
            {
                f.read_bar(bar, offset, data);
                return true;
            }
        }
        data.fill(0xFF);
        false
    }

    /// A memory write. False when nothing is at `address`.
    pub fn write_memory(&self, address: u64, data: &[u8]) -> bool {
        if Self::is_ecam(address) {
            let (bus, device, function, offset) = Self::ecam(address - ECAM_START);
            self.write_config(bus, device, function, offset, data);
            return true;
        }
        for slot in self.slots.iter().flatten() {
            if let Ok(mut f) = slot.lock()
                && let Some((bar, offset)) = f.bar_at(address)
            {
                f.write_bar(bar, offset, data);
                return true;
            }
        }
        false
    }
}

impl Default for PciBus {
    fn default() -> Self {
        Self::new()
    }
}

/// The configuration ports: the address at 0 to 3, the data at 4 to 7.
impl BusDevice for PciBus {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        data.fill(0xFF);
        match offset {
            0..4 => {
                let bytes = self.address.to_le_bytes();
                for (i, byte) in data.iter_mut().enumerate() {
                    if let Some(b) = usize::try_from(offset).ok().and_then(|o| bytes.get(o + i)) {
                        *byte = *b;
                    }
                }
            }
            4..8 if self.address & 0x8000_0000 != 0 => {
                let a = self.address;
                #[allow(clippy::cast_possible_truncation)] // 0 to 3
                let register = (a & 0xFC) as usize + (offset - 4) as usize;
                self.read_config(
                    (a >> 16) & 0xFF,
                    (a >> 11) & 0x1F,
                    (a >> 8) & 0x7,
                    register,
                    data,
                );
            }
            _ => {}
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        match offset {
            0..4 => {
                let mut bytes = self.address.to_le_bytes();
                for (i, value) in data.iter().enumerate() {
                    if let Some(b) = usize::try_from(offset)
                        .ok()
                        .and_then(|o| bytes.get_mut(o + i))
                    {
                        *b = *value;
                    }
                }
                self.address = u32::from_le_bytes(bytes);
            }
            4..8 if self.address & 0x8000_0000 != 0 => {
                let a = self.address;
                #[allow(clippy::cast_possible_truncation)] // 0 to 3
                let register = (a & 0xFC) as usize + (offset - 4) as usize;
                self.write_config(
                    (a >> 16) & 0xFF,
                    (a >> 11) & 0x1F,
                    (a >> 8) & 0x7,
                    register,
                    data,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pci::config::COMMAND_MEMORY;

    struct Card {
        config: Config,
        written: Vec<(usize, u64, Vec<u8>)>,
    }

    impl PciDevice for Card {
        fn read_config(&mut self, offset: usize, data: &mut [u8]) {
            self.config.read(offset, data);
        }
        fn write_config(&mut self, offset: usize, data: &[u8]) {
            self.config.write(offset, data);
        }
        fn bar_at(&self, address: u64) -> Option<(usize, u64)> {
            self.config.bar_at(address)
        }
        fn read_bar(&mut self, bar: usize, offset: u64, data: &mut [u8]) {
            #[allow(clippy::cast_possible_truncation)]
            data.fill(bar as u8 + offset as u8);
        }
        fn write_bar(&mut self, bar: usize, offset: u64, data: &[u8]) {
            self.written.push((bar, offset, data.to_vec()));
        }
    }

    fn bus_with_card() -> (PciBus, Arc<Mutex<Card>>) {
        let mut config = Config::new(&Identity {
            vendor: 0x1AF4,
            device: 0x1044,
            revision: 1,
            class: 0x00_FF_00,
            subsystem_vendor: 0x1AF4,
            subsystem: 4,
        });
        assert!(config.add_bar(0, 0xC000_0000, 0x4000));
        let card = Arc::new(Mutex::new(Card {
            config,
            written: Vec::new(),
        }));
        let mut bus = PciBus::new();
        assert_eq!(bus.add(card.clone()), Some(1));
        (bus, card)
    }

    fn port_read32(bus: &mut PciBus, device: u32, register: u32) -> u32 {
        let address = 0x8000_0000 | (device << 11) | register;
        bus.write(0, &address.to_le_bytes());
        let mut data = [0u8; 4];
        bus.read(4, &mut data);
        u32::from_le_bytes(data)
    }

    #[test]
    fn both_ways_reach_the_same_registers() {
        let (mut bus, _) = bus_with_card();
        assert_eq!(port_read32(&mut bus, 0, 0), 0x0008_1B36);
        assert_eq!(port_read32(&mut bus, 1, 0), 0x1044_1AF4);
        assert_eq!(port_read32(&mut bus, 2, 0), 0xFFFF_FFFF);
        let mut data = [0u8; 4];
        assert!(bus.read_memory(ECAM_START + (1 << 15), &mut data));
        assert_eq!(u32::from_le_bytes(data), 0x1044_1AF4);
        // Function 1 and bus 1 have nothing.
        assert!(bus.read_memory(ECAM_START + (1 << 15) + (1 << 12), &mut data));
        assert_eq!(data, [0xFF; 4]);
        // The address port reads back; the data port is dead without
        // the enable bit.
        bus.write(0, &0x0000_0800u32.to_le_bytes());
        bus.read(4, &mut data);
        assert_eq!(data, [0xFF; 4]);
        bus.read(0, &mut data);
        assert_eq!(u32::from_le_bytes(data), 0x0000_0800);
    }

    #[test]
    fn a_memory_access_goes_to_the_function_whose_bar_is_there() {
        let (mut bus, card) = bus_with_card();
        let mut data = [0u8; 2];
        // Decoding is off.
        assert!(!bus.read_memory(0xC000_0010, &mut data));
        bus.write(0, &(0x8000_0000u32 | (1 << 11) | 4).to_le_bytes());
        bus.write(4, &COMMAND_MEMORY.to_le_bytes());
        assert!(bus.read_memory(0xC000_0010, &mut data));
        assert_eq!(data, [0x10, 0x10]);
        assert!(bus.write_memory(0xC000_3FFC, &[1, 2, 3, 4]));
        assert!(!bus.write_memory(0xC000_4000, &[1]));
        assert_eq!(
            card.lock().unwrap().written,
            [(0, 0x3FFC, vec![1, 2, 3, 4])]
        );
    }

    #[test]
    fn the_bus_fills_up() {
        let (mut bus, card) = bus_with_card();
        for _ in 2..SLOTS {
            assert!(bus.add(card.clone()).is_some());
        }
        assert!(bus.add(card).is_none());
    }
}
