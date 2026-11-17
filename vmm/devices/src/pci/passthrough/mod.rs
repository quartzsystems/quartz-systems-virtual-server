// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A host's PCI function given to the partition whole
//! (`docs/PHYSICALIO.md`, `docs/VMM.md`). The function is reached
//! through [`Function`]: VFIO's device in the VMM (`crate::sys::vfio`),
//! a double in a test.
//!
//! - Its memory BARs are placed in the partition's PCI memory space
//!   from the top down, each on its own size, and mapped straight into
//!   the guest ([`Passthrough::mappings`]) but for the pages of the
//!   MSI-X table and pending bits, which the VMM answers. An I/O BAR is
//!   not given.
//! - Its configuration space is the VMM's to present ([`space`]).
//! - Its interrupts are message interrupts only: MSI-X through the
//!   VMM's table, or MSI, each vector an interrupt number of its own
//!   whose eventfd the device's driver in the host kernel writes, so an
//!   interrupt reaches the guest without leaving the kernel.
//!
//! Every configuration and BAR access is guest input.

pub mod space;

use std::io;
use std::sync::Arc;

use vmm_sys_util::eventfd::EventFd;

use crate::pci::bus::PciDevice;
use crate::pci::msix::{MAX_VECTORS, Msix};
use crate::platform::{MsiRoute, Platform};
use space::{Bar, CAP_MSI, CAP_MSIX, COMMAND_MASTER, Space};

/// The page a mapping is made of.
pub const PAGE: u64 = 4096;
/// The most MSI vectors.
const MAX_MSI: u32 = 32;

/// The kinds of message interrupt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interrupts {
    /// MSI.
    Msi,
    /// MSI-X.
    Msix,
}

/// A BAR's region as the host's kernel describes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Region {
    /// Its size; 0 when the BAR is not there.
    pub size: u64,
    /// Whether it can be mapped into the VMM.
    pub mappable: bool,
}

/// What the VMM reaches of the host's function.
pub trait Function: Send {
    /// Read the function's own configuration space at `offset`; what
    /// cannot be read is all ones.
    fn config_read(&self, offset: u16, data: &mut [u8]);
    /// Write it.
    fn config_write(&self, offset: u16, data: &[u8]);
    /// Read BAR `bar` at `offset`, where it is not mapped.
    fn bar_read(&self, bar: usize, offset: u64, data: &mut [u8]);
    /// Write it.
    fn bar_write(&self, bar: usize, offset: u64, data: &[u8]);
    /// Have vector `i` of `kind` write `fds[i]`; none turns them off.
    fn interrupts(&self, kind: Interrupts, fds: &[&EventFd]) -> io::Result<()>;
}

/// Where BARs go: from the top of a window down, each aligned on its
/// own size.
#[derive(Clone, Copy, Debug)]
pub struct Placer {
    low: u64,
    top: u64,
}

impl Placer {
    /// The window from `low` to `top`.
    pub fn new(low: u64, top: u64) -> Self {
        Self { low, top }
    }

    /// The place of a BAR of `size`; none when it does not fit.
    pub fn place(&mut self, size: u64) -> Option<u64> {
        let size = size.max(PAGE).checked_next_power_of_two()?;
        let at = self.top.checked_sub(size)? & !(size - 1);
        if at < self.low {
            return None;
        }
        self.top = at;
        Some(at)
    }
}

/// A piece of a BAR mapped straight into the guest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    /// The BAR.
    pub bar: usize,
    /// Its offset in the BAR, a page multiple.
    pub offset: u64,
    /// Its bytes, a page multiple.
    pub len: u64,
    /// Where it is in the guest.
    pub guest: u64,
}

/// Where a part of a BAR is: the BAR, its offset and bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Part {
    bar: usize,
    offset: u64,
    bytes: u64,
}

impl Part {
    fn holds(&self, bar: usize, offset: u64) -> Option<u64> {
        (bar == self.bar && offset >= self.offset && offset - self.offset < self.bytes)
            .then(|| offset - self.offset)
    }

    /// Its pages, as a start and an end.
    fn pages(&self) -> (u64, u64) {
        let start = self.offset & !(PAGE - 1);
        let end = self
            .offset
            .saturating_add(self.bytes)
            .saturating_add(PAGE - 1)
            & !(PAGE - 1);
        (start, end)
    }
}

struct MsixState {
    msix: Msix,
    table: Part,
    pba: Part,
    on: bool,
}

struct MsiState {
    vectors: Vec<(u32, EventFd)>,
    max: u32,
    on: u32,
}

/// The function on the partition's bus.
pub struct Passthrough {
    function: Box<dyn Function>,
    platform: Arc<dyn Platform>,
    space: Space,
    bars: Vec<Bar>,
    regions: [Region; 6],
    msix: Option<MsixState>,
    msi: Option<MsiState>,
    dma: bool,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn refuse(why: impl Into<String>) -> io::Error {
    io::Error::other(why.into())
}

impl Passthrough {
    /// The function `function`, whose BARs' regions are `regions`, its
    /// BARs placed by `placer`.
    pub fn new(
        function: Box<dyn Function>,
        regions: [Region; 6],
        platform: Arc<dyn Platform>,
        placer: &mut Placer,
    ) -> io::Result<Self> {
        let mut dev = [0u8; space::SPACE_BYTES];
        function.config_read(0, &mut dev);
        if le16(&dev, 0) == u16::MAX {
            return Err(refuse("its configuration space cannot be read"));
        }
        if dev[0x0E] & 0x7F != 0 {
            return Err(refuse("it is not an endpoint: a bridge is not given"));
        }
        let mut bars = Vec::new();
        let mut i = 0;
        while i < 6 {
            let raw = le32(&dev, 0x10 + 4 * i);
            let wide = raw & 0b111 == 0b100;
            let size = regions[i].size;
            if raw & 1 == 1 || size == 0 {
                i += 1;
                continue;
            }
            if !size.is_power_of_two() {
                return Err(refuse(format!("BAR {i} has a size of {size}")));
            }
            let address = placer
                .place(size)
                .ok_or_else(|| refuse(format!("BAR {i} of {size} bytes does not fit")))?;
            bars.push(Bar {
                index: i,
                size,
                address,
                wide,
                prefetch: raw & 0b1000 != 0,
            });
            i += if wide { 2 } else { 1 };
        }
        let caps = space::kept_capabilities(&dev);
        let bar_bytes = |index: usize| bars.iter().find(|b| b.index == index).map(|b| b.size);
        let msix = match caps.iter().find(|k| k.id == CAP_MSIX) {
            Some(k) => {
                let count = usize::from(le16(&dev, k.at + 2) & 0x7FF) + 1;
                let vectors = count.min(MAX_VECTORS);
                let (table, pba) = (le32(&dev, k.at + 4), le32(&dev, k.at + 8));
                let part = |reg: u32, bytes: usize| -> io::Result<Part> {
                    let bar = (reg & 7) as usize;
                    let offset = u64::from(reg & !7);
                    let bytes = bytes as u64;
                    match bar_bytes(bar) {
                        Some(size) if offset.checked_add(bytes).is_some_and(|e| e <= size) => {
                            Ok(Part { bar, offset, bytes })
                        }
                        _ => Err(refuse("its MSI-X table is not inside a BAR")),
                    }
                };
                Some(MsixState {
                    msix: Msix::new(platform.clone(), vectors)?,
                    table: part(table, vectors * 16)?,
                    pba: part(pba, vectors.div_ceil(64) * 8)?,
                    on: false,
                })
            }
            None => None,
        };
        let msi = caps.iter().find(|k| k.id == CAP_MSI).map(|k| MsiState {
            vectors: Vec::new(),
            max: (1u32 << ((le16(&dev, k.at + 2) >> 1) & 7)).min(MAX_MSI),
            on: 0,
        });
        if msix.is_none() && msi.is_none() {
            return Err(refuse("it signals no message interrupt"));
        }
        let vectors = msix
            .as_ref()
            .map_or(0, |m| u16::try_from(m.msix.len()).unwrap_or(0));
        let space = Space::new(&dev, &bars, vectors);
        // Off at the device until the guest says: `INTx` off for good.
        function.config_write(0x04, &space.device_command().to_le_bytes());
        Ok(Self {
            function,
            platform,
            space,
            bars,
            regions,
            msix,
            msi,
            dma: false,
        })
    }

    /// The BARs as placed.
    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }

    /// The pieces of the BARs that are mapped straight into the guest:
    /// each mappable BAR of a page or more, less the pages of the MSI-X
    /// table and pending bits.
    pub fn mappings(&self) -> Vec<Mapping> {
        let mut out = Vec::new();
        for b in &self.bars {
            if !self.regions[b.index].mappable || b.size < PAGE {
                continue;
            }
            let mut holes: Vec<(u64, u64)> = self
                .msix
                .iter()
                .flat_map(|m| [m.table, m.pba])
                .filter(|p| p.bar == b.index)
                .map(|p| p.pages())
                .collect();
            holes.sort_unstable();
            let mut at = 0;
            for (start, end) in holes {
                if start > at {
                    out.push(Mapping {
                        bar: b.index,
                        offset: at,
                        len: start - at,
                        guest: b.address + at,
                    });
                }
                at = at.max(end);
            }
            if at < b.size {
                out.push(Mapping {
                    bar: b.index,
                    offset: at,
                    len: b.size - at,
                    guest: b.address + at,
                });
            }
        }
        out
    }

    /// The partition's memory is mapped for the device's DMA: bus
    /// mastering may reach the device from now on.
    pub fn dma_ready(&mut self) {
        self.dma = true;
        self.function
            .config_write(0x04, &self.space.device_command().to_le_bytes());
    }

    /// The command as the device is to have it: bus mastering held
    /// back until the mappings exist.
    fn command(&self) -> u16 {
        let c = self.space.device_command();
        if self.dma { c } else { c & !COMMAND_MASTER }
    }

    fn update_msix(&mut self, control: u16) {
        let Some(m) = self.msix.as_mut() else {
            return;
        };
        m.msix.set_control(control);
        let enable = control & crate::pci::msix::CONTROL_ENABLE != 0;
        if enable && !m.on {
            if let Some(msi) = self.msi.as_mut()
                && msi.on > 0
            {
                let _ = self.function.interrupts(Interrupts::Msi, &[]);
                msi.on = 0;
            }
            let fds: Vec<EventFd> = (0..m.msix.len())
                .filter_map(|i| m.msix.vector_fd(u16::try_from(i).ok()?))
                .collect();
            let refs: Vec<&EventFd> = fds.iter().collect();
            m.on = self.function.interrupts(Interrupts::Msix, &refs).is_ok();
        } else if !enable && m.on {
            let _ = self.function.interrupts(Interrupts::Msix, &[]);
            m.on = false;
        }
    }

    fn update_msi(&mut self) {
        let (Some(k), Some(msi)) = (self.space.capability(CAP_MSI), self.msi.as_mut()) else {
            return;
        };
        let control = self.space.shadow16(k.at + 2);
        let wide = control & (1 << 7) != 0;
        let enabled = control & 1 != 0 && !self.msix.as_ref().is_some_and(|m| m.on);
        let wanted = if enabled {
            (1u32 << ((control >> 4) & 7)).min(msi.max)
        } else {
            0
        };
        let low = u64::from(self.space.shadow32(k.at + 4));
        let high = if wide {
            u64::from(self.space.shadow32(k.at + 8))
        } else {
            0
        };
        let data = u32::from(self.space.shadow16(if wide { k.at + 12 } else { k.at + 8 }));
        let count = |v: &Vec<(u32, EventFd)>| u32::try_from(v.len()).unwrap_or(u32::MAX);
        while count(&msi.vectors) < wanted {
            let Some(gsi) = self.platform.allocate_gsi() else {
                break;
            };
            let Ok(fd) = EventFd::new(0) else {
                break;
            };
            if self.platform.irqfd(gsi, &fd).is_err() {
                break;
            }
            msi.vectors.push((gsi, fd));
        }
        let wanted = wanted.min(count(&msi.vectors));
        for (i, (gsi, _)) in (0u32..).zip(&msi.vectors) {
            let route = (i < wanted).then(|| MsiRoute {
                address: (high << 32) | low,
                data: (data & !(wanted.max(1) - 1)) | i,
            });
            let _ = self.platform.route_msi(*gsi, route);
        }
        if wanted != msi.on {
            let fds: Vec<&EventFd> = msi.vectors[..wanted as usize]
                .iter()
                .map(|(_, fd)| fd)
                .collect();
            if wanted == 0 || self.function.interrupts(Interrupts::Msi, &[]).is_ok() {
                let _ = self.function.interrupts(Interrupts::Msi, &fds);
            }
            msi.on = wanted;
        }
    }

    /// The BAR at `index`.
    fn bar(&self, index: usize) -> Option<&Bar> {
        self.bars.iter().find(|b| b.index == index)
    }
}

impl PciDevice for Passthrough {
    fn read_config(&mut self, offset: usize, data: &mut [u8]) {
        for (i, chunk) in data.chunks_mut(8).enumerate() {
            let Some(at) = offset.checked_add(i * 8) else {
                chunk.fill(0);
                continue;
            };
            let mut live = [0u8; 8];
            if let Ok(o) = u16::try_from(at)
                && at < space::SPACE_BYTES
            {
                let n = chunk.len().min(space::SPACE_BYTES - at);
                self.function.config_read(o, &mut live[..n]);
            }
            self.space
                .read(at, chunk, |byte| live.get(byte - at).copied().unwrap_or(0));
        }
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) {
        let changed = self.space.write(offset, data);
        if changed.command.is_some() {
            self.function
                .config_write(0x04, &self.command().to_le_bytes());
        }
        if let Some(bits) = changed.status {
            self.function.config_write(0x06, &bits.to_le_bytes());
        }
        if let Some(control) = changed.msix {
            self.update_msix(control);
        }
        if changed.msi {
            self.update_msi();
        }
    }

    fn bar_at(&self, address: u64) -> Option<(usize, u64)> {
        self.bars
            .iter()
            .find(|b| address >= b.address && address - b.address < b.size)
            .map(|b| (b.index, address - b.address))
    }

    fn read_bar(&mut self, bar: usize, offset: u64, data: &mut [u8]) {
        let inside = self.bar(bar).is_some_and(|b| {
            offset
                .checked_add(data.len() as u64)
                .is_some_and(|e| e <= b.size)
        });
        if !inside {
            data.fill(0xFF);
            return;
        }
        if let Some(m) = &self.msix {
            if let Some(at) = m.table.holds(bar, offset) {
                return m.msix.read_table(at, data);
            }
            if let Some(at) = m.pba.holds(bar, offset) {
                return m.msix.read_pending(at, data);
            }
        }
        self.function.bar_read(bar, offset, data);
    }

    fn write_bar(&mut self, bar: usize, offset: u64, data: &[u8]) {
        let inside = self.bar(bar).is_some_and(|b| {
            offset
                .checked_add(data.len() as u64)
                .is_some_and(|e| e <= b.size)
        });
        if !inside {
            return;
        }
        if let Some(m) = self.msix.as_mut() {
            if let Some(at) = m.table.holds(bar, offset) {
                return m.msix.write_table(at, data);
            }
            if m.pba.holds(bar, offset).is_some() {
                return;
            }
        }
        self.function.bar_write(bar, offset, data);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::platform::fake::Recorder;
    use space::tests::device_space;
    use std::sync::Mutex;

    /// What the double saw.
    #[derive(Default)]
    pub(crate) struct Seen {
        pub config: Vec<(u16, Vec<u8>)>,
        pub bar: Vec<(usize, u64, usize)>,
        pub interrupts: Vec<(Interrupts, usize)>,
    }

    pub(crate) struct Double {
        pub space: Vec<u8>,
        pub seen: Arc<Mutex<Seen>>,
    }

    impl Function for Double {
        fn config_read(&self, offset: u16, data: &mut [u8]) {
            for (i, d) in data.iter_mut().enumerate() {
                *d = self
                    .space
                    .get(usize::from(offset) + i)
                    .copied()
                    .unwrap_or(0xFF);
            }
        }
        fn config_write(&self, offset: u16, data: &[u8]) {
            self.seen
                .lock()
                .unwrap()
                .config
                .push((offset, data.to_vec()));
        }
        fn bar_read(&self, bar: usize, offset: u64, data: &mut [u8]) {
            self.seen
                .lock()
                .unwrap()
                .bar
                .push((bar, offset, data.len()));
            data.fill(0x5A);
        }
        fn bar_write(&self, bar: usize, offset: u64, data: &[u8]) {
            self.seen
                .lock()
                .unwrap()
                .bar
                .push((bar, offset, data.len()));
        }
        fn interrupts(&self, kind: Interrupts, fds: &[&EventFd]) -> io::Result<()> {
            self.seen.lock().unwrap().interrupts.push((kind, fds.len()));
            Ok(())
        }
    }

    pub(crate) fn regions() -> [Region; 6] {
        let mut r = [Region::default(); 6];
        r[0] = Region {
            size: 0x4000,
            mappable: true,
        };
        r[2] = Region {
            size: 0x100,
            mappable: false,
        };
        r
    }

    pub(crate) fn device(seen: Arc<Mutex<Seen>>) -> Passthrough {
        let d = Double {
            space: device_space(),
            seen,
        };
        let mut placer = Placer::new(0xC010_0000, 0xE800_0000);
        Passthrough::new(
            Box::new(d),
            regions(),
            Arc::new(Recorder::default()),
            &mut placer,
        )
        .unwrap()
    }

    #[test]
    fn bars_are_placed_from_the_top_and_mapped_but_for_the_msix_pages() {
        let p = device(Arc::default());
        assert_eq!(p.bars().len(), 1, "the I/O BAR is not given");
        assert_eq!(p.bars()[0].address, 0xE7FF_C000);
        assert_eq!(
            p.mappings(),
            [Mapping {
                bar: 0,
                offset: 0,
                len: 0x2000,
                guest: 0xE7FF_C000
            },]
        );
        let mut placer = Placer::new(0xE000_0000, 0xE800_0000);
        assert_eq!(placer.place(0x100), Some(0xE7FF_F000));
        assert_eq!(placer.place(0x10_0000), Some(0xE7E0_0000));
        assert_eq!(placer.place(0x1000_0000), None);
    }

    #[test]
    fn the_msix_table_is_the_vmm_s_and_turns_the_device_s_vectors_on() {
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let mut p = device(seen.clone());
        // An entry of the table is answered here, not by the device.
        p.write_bar(0, 0x2000, &[0, 0, 0xE0, 0xFE]);
        let mut entry = [0u8; 4];
        p.read_bar(0, 0x2000, &mut entry);
        assert_eq!(entry, [0, 0, 0xE0, 0xFE]);
        assert!(seen.lock().unwrap().bar.is_empty());
        // Elsewhere in the BAR is the device's.
        let mut doorbell = [0u8; 4];
        p.read_bar(0, 0x1000, &mut doorbell);
        assert_eq!(seen.lock().unwrap().bar, [(0, 0x1000, 4)]);
        p.write_config(0xC2, &0x8000u16.to_le_bytes());
        assert_eq!(
            seen.lock().unwrap().interrupts,
            [(Interrupts::Msix, MAX_VECTORS)]
        );
        p.write_config(0xC2, &0u16.to_le_bytes());
        assert_eq!(seen.lock().unwrap().interrupts[1], (Interrupts::Msix, 0));
    }

    #[test]
    fn msi_is_set_up_from_its_capability() {
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let mut p = device(seen.clone());
        p.write_config(0x54, &0xFEE0_0000u32.to_le_bytes());
        p.write_config(0x5C, &0x0041u16.to_le_bytes());
        // Enable with four messages (MME 2).
        p.write_config(0x52, &0x0021u16.to_le_bytes());
        let i = seen.lock().unwrap().interrupts.clone();
        assert_eq!(i.last(), Some(&(Interrupts::Msi, 4)), "{i:?}");
        p.write_config(0x52, &0u16.to_le_bytes());
        assert_eq!(
            seen.lock().unwrap().interrupts.last(),
            Some(&(Interrupts::Msi, 0))
        );
    }

    #[test]
    fn bus_mastering_waits_for_the_mappings() {
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let mut p = device(seen.clone());
        p.write_config(0x04, &0x0006u16.to_le_bytes());
        let last = |s: &Arc<Mutex<Seen>>| {
            let c = s.lock().unwrap().config.last().cloned().unwrap();
            u16::from_le_bytes([c.1[0], c.1[1]])
        };
        assert_eq!(last(&seen) & COMMAND_MASTER, 0);
        assert_ne!(last(&seen) & space::COMMAND_INTX_OFF, 0);
        p.dma_ready();
        assert_ne!(last(&seen) & COMMAND_MASTER, 0);
    }

    #[test]
    fn accesses_outside_a_bar_reach_nothing() {
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let mut p = device(seen.clone());
        let mut d = [0u8; 8];
        p.read_bar(0, 0x3FFC, &mut d);
        assert_eq!(d, [0xFF; 8]);
        p.read_bar(4, 0, &mut d);
        p.write_bar(0, u64::MAX - 2, &d);
        p.write_bar(5, 0, &d);
        assert!(seen.lock().unwrap().bar.is_empty());
        assert_eq!(p.bar_at(0xE7FF_C000), Some((0, 0)));
        assert_eq!(p.bar_at(0xE800_0000), None);
    }

    #[test]
    fn a_bridge_or_a_function_without_message_interrupts_is_refused() {
        let mut bridge = device_space();
        bridge[0x0E] = 1;
        let mut no_msi = device_space();
        no_msi[0x06] = 0;
        for s in [bridge, no_msi] {
            let d = Double {
                space: s,
                seen: Arc::default(),
            };
            let mut placer = Placer::new(0xC010_0000, 0xE800_0000);
            assert!(
                Passthrough::new(
                    Box::new(d),
                    regions(),
                    Arc::new(Recorder::default()),
                    &mut placer
                )
                .is_err()
            );
        }
    }
}
