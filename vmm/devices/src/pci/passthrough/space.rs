// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A passed function's configuration space as the guest sees it. The
//! VMM presents it; the device's own is behind it and is reached only
//! where this says:
//!
//! | Bytes | The guest reads | A guest write |
//! |---|---|---|
//! | IDs, class, subsystem, header | the device's, the multifunction bit cleared | ignored |
//! | command | what it wrote | memory, bus master, parity and SERR reach the device, `INTx` is always off there, I/O never on |
//! | status | the device's | its write-one-to-clear bits reach the device |
//! | BARs | the VMM's places, sized as the device's | sizing is answered; an address is ignored |
//! | I/O BARs, the expansion ROM, the interrupt pin | zero | ignored |
//! | power management | the device's, with the power state it wrote | kept here: the device stays in D0 |
//! | PCI Express | the device's | ignored |
//! | MSI, MSI-X | the VMM's | the VMM's: they set the interrupts up |
//! | every other capability, device-specific bytes, extended space | zero | ignored |
//!
//! Every write is guest input: any offset, any length, any value.

/// Bytes of the space presented; extended space reads as zero.
pub const SPACE_BYTES: usize = 256;

/// The capability IDs kept.
pub const CAP_PM: u8 = 0x01;
/// MSI.
pub const CAP_MSI: u8 = 0x05;
/// PCI Express.
pub const CAP_EXP: u8 = 0x10;
/// MSI-X.
pub const CAP_MSIX: u8 = 0x11;

const COMMAND: usize = 0x04;
const STATUS: usize = 0x06;
const HEADER_TYPE: usize = 0x0E;
const BAR0: usize = 0x10;
const BAR_END: usize = 0x28;
const CAPABILITIES: usize = 0x34;
const INTERRUPT_LINE: usize = 0x3C;

/// Command bits passed to the device.
pub const COMMAND_TO_DEVICE: u16 = (1 << 1) | (1 << 2) | (1 << 6) | (1 << 8);
/// Command: bus master.
pub const COMMAND_MASTER: u16 = 1 << 2;
/// Command: `INTx` disabled, always so at the device.
pub const COMMAND_INTX_OFF: u16 = 1 << 10;
/// The command bits the guest may set.
const COMMAND_WRITABLE: u16 = COMMAND_TO_DEVICE | COMMAND_INTX_OFF;
/// The status bits that are cleared by writing one.
pub const STATUS_RW1C: u16 = 0xF900;

/// The most capabilities followed in the device's list.
const MOST_CAPABILITIES: usize = 48;
/// Bytes of a PCI Express capability (version 2).
const EXP_BYTES: usize = 0x3C;
/// Bytes of a power management capability.
const PM_BYTES: usize = 8;
/// Bytes of an MSI-X capability.
const MSIX_BYTES: usize = 12;

/// How a byte is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Zero, writes ignored.
    Hidden,
    /// The shadow's, writable where the mask says.
    Shadow,
    /// The device's, read each time; writes ignored.
    Live,
}

/// A memory BAR as presented.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    /// Its index, 0 to 5; a 64-bit BAR takes the next too.
    pub index: usize,
    /// Its size, a power of two.
    pub size: u64,
    /// Where the VMM put it in the guest.
    pub address: u64,
    /// Whether it is 64-bit.
    pub wide: bool,
    /// Whether it is prefetchable.
    pub prefetch: bool,
}

/// One BAR register's state.
#[derive(Clone, Copy, Debug, Default)]
struct BarReg {
    /// The BAR this dword is part of; none for an empty or hidden one.
    bar: Option<Bar>,
    /// The upper dword of a 64-bit BAR.
    high: bool,
    /// The guest wrote all ones: the next read is the size mask.
    sizing: bool,
}

/// A capability kept, where it is in the space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept {
    /// Its ID.
    pub id: u8,
    /// Its offset.
    pub at: usize,
    /// Its bytes.
    pub len: usize,
}

/// What a guest write changed, for the device to act on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    /// The command register, as the device is to have it.
    pub command: Option<u16>,
    /// Status bits to clear at the device.
    pub status: Option<u16>,
    /// The MSI capability was written.
    pub msi: bool,
    /// The MSI-X control word, as the guest has it now.
    pub msix: Option<u16>,
}

/// The space.
pub struct Space {
    shadow: [u8; SPACE_BYTES],
    writable: [u8; SPACE_BYTES],
    kind: [Kind; SPACE_BYTES],
    bars: [BarReg; 6],
    kept: Vec<Kept>,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([
        b.get(at).copied().unwrap_or(0),
        b.get(at + 1).copied().unwrap_or(0),
    ])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([
        b.get(at).copied().unwrap_or(0),
        b.get(at + 1).copied().unwrap_or(0),
        b.get(at + 2).copied().unwrap_or(0),
        b.get(at + 3).copied().unwrap_or(0),
    ])
}

/// The bytes of an MSI capability with control word `control`.
pub fn msi_bytes(control: u16) -> usize {
    let wide = control & (1 << 7) != 0;
    let masks = control & (1 << 8) != 0;
    match (wide, masks) {
        (false, false) => 10,
        (true, false) => 14,
        (false, true) => 20,
        (true, true) => 24,
    }
}

/// The capabilities of a device's space `dev` that are kept, in list
/// order: power management, PCI Express, MSI, MSI-X, each once, each
/// whole inside the space. The list is followed a bounded number of
/// times and never into the header.
pub fn kept_capabilities(dev: &[u8]) -> Vec<Kept> {
    let mut out: Vec<Kept> = Vec::new();
    if le16(dev, STATUS) & (1 << 4) == 0 {
        return out;
    }
    let mut at = usize::from(dev.get(CAPABILITIES).copied().unwrap_or(0) & 0xFC);
    let mut seen = 0;
    while (0x40..SPACE_BYTES).contains(&at) && seen < MOST_CAPABILITIES {
        let id = dev.get(at).copied().unwrap_or(0);
        let next = usize::from(dev.get(at + 1).copied().unwrap_or(0) & 0xFC);
        let len = match id {
            CAP_PM => Some(PM_BYTES),
            CAP_EXP => Some(EXP_BYTES),
            CAP_MSI => Some(msi_bytes(le16(dev, at + 2))),
            CAP_MSIX => Some(MSIX_BYTES),
            _ => None,
        };
        if let Some(len) = len
            && at + len <= SPACE_BYTES
            && !out.iter().any(|k| k.id == id)
            && !out.iter().any(|k| at < k.at + k.len && k.at < at + len)
        {
            out.push(Kept { id, at, len });
        }
        at = next;
        seen += 1;
    }
    out
}

impl Space {
    /// The space presented for a device whose own space is `dev` (256
    /// bytes, shorter reads as zeros), with `bars` placed and an MSI-X
    /// table of `msix_vectors` vectors.
    pub fn new(dev: &[u8], bars: &[Bar], msix_vectors: u16) -> Self {
        let mut s = Self {
            shadow: [0; SPACE_BYTES],
            writable: [0; SPACE_BYTES],
            kind: [Kind::Hidden; SPACE_BYTES],
            bars: [BarReg::default(); 6],
            kept: kept_capabilities(dev),
        };
        let copy = |s: &mut Self, from: usize, to: usize| {
            for at in from..to {
                s.shadow[at] = dev.get(at).copied().unwrap_or(0);
                s.kind[at] = Kind::Shadow;
            }
        };
        // IDs, class, cache line, latency, header, subsystem, min/max.
        copy(&mut s, 0x00, 0x04);
        copy(&mut s, 0x08, 0x10);
        copy(&mut s, 0x2C, 0x30);
        copy(&mut s, 0x3E, 0x40);
        s.shadow[HEADER_TYPE] &= 0x7F;
        s.shadow[0x0F] = 0;
        for at in [0x0C, 0x0D] {
            s.writable[at] = 0xFF;
        }
        // Command, status.
        for at in COMMAND..COMMAND + 2 {
            s.kind[at] = Kind::Shadow;
        }
        s.writable[COMMAND..COMMAND + 2].copy_from_slice(&COMMAND_WRITABLE.to_le_bytes());
        s.shadow[COMMAND..COMMAND + 2].copy_from_slice(&COMMAND_INTX_OFF.to_le_bytes());
        for at in STATUS..STATUS + 2 {
            s.kind[at] = Kind::Live;
        }
        // BARs: the dwords are answered by `bars`.
        for b in bars.iter().filter(|b| b.index < 6) {
            s.bars[b.index] = BarReg {
                bar: Some(*b),
                high: false,
                sizing: false,
            };
            if b.wide && b.index < 5 {
                s.bars[b.index + 1] = BarReg {
                    bar: Some(*b),
                    high: true,
                    sizing: false,
                };
            }
        }
        // The interrupt line is the guest's scratch.
        s.kind[INTERRUPT_LINE] = Kind::Shadow;
        s.writable[INTERRUPT_LINE] = 0xFF;
        // The capabilities kept, chained in order.
        s.kind[CAPABILITIES] = Kind::Shadow;
        s.shadow[CAPABILITIES] = s
            .kept
            .first()
            .map_or(0, |k| u8::try_from(k.at).unwrap_or(0));
        for i in 0..s.kept.len() {
            let k = s.kept[i];
            let next = s
                .kept
                .get(i + 1)
                .map_or(0, |n| u8::try_from(n.at).unwrap_or(0));
            s.keep(k, next, dev, msix_vectors);
        }
        s
    }

    /// Present the kept capability `k`, with `next` after it.
    fn keep(&mut self, k: Kept, next: u8, dev: &[u8], msix_vectors: u16) {
        let s = self;
        for at in k.at..k.at + k.len {
            s.shadow[at] = dev.get(at).copied().unwrap_or(0);
        }
        s.kind[k.at] = Kind::Shadow;
        s.kind[k.at + 1] = Kind::Shadow;
        s.shadow[k.at + 1] = next;
        match k.id {
            CAP_PM => {
                for at in k.at + 2..k.at + k.len {
                    s.kind[at] = Kind::Live;
                }
                // The control and status word is the guest's.
                for at in k.at + 4..k.at + 6 {
                    s.kind[at] = Kind::Shadow;
                }
                s.shadow[k.at + 4] &= !0x03;
                s.writable[k.at + 4] = 0x03;
            }
            CAP_EXP => {
                for at in k.at + 2..k.at + k.len {
                    s.kind[at] = Kind::Live;
                }
            }
            CAP_MSI => {
                for at in k.at + 2..k.at + k.len {
                    s.kind[at] = Kind::Shadow;
                }
                let control = le16(&s.shadow, k.at + 2);
                // Enable and the messages enabled; off to begin with.
                s.writable[k.at + 2] = 0x71;
                let off = control & !0x0071;
                s.shadow[k.at + 2..k.at + 4].copy_from_slice(&off.to_le_bytes());
                let wide = control & (1 << 7) != 0;
                let masks = control & (1 << 8) != 0;
                let data = if wide { k.at + 12 } else { k.at + 8 };
                for at in k.at + 4..k.at + 8 {
                    s.writable[at] = 0xFF;
                }
                s.writable[k.at + 4] = 0xFC;
                if wide {
                    for at in k.at + 8..k.at + 12 {
                        s.writable[at] = 0xFF;
                    }
                }
                for at in data..data + 2 {
                    s.writable[at] = 0xFF;
                }
                if masks {
                    for at in data + 4..data + 8 {
                        s.writable[at] = 0xFF;
                    }
                }
                for at in k.at + 4..k.at + k.len {
                    s.shadow[at] = 0;
                }
            }
            CAP_MSIX => {
                for at in k.at + 2..k.at + k.len {
                    s.kind[at] = Kind::Shadow;
                }
                let table = msix_vectors.saturating_sub(1) & 0x07FF;
                s.shadow[k.at + 2..k.at + 4].copy_from_slice(&table.to_le_bytes());
                // Enable and function mask.
                s.writable[k.at + 3] = 0xC0;
            }
            _ => {}
        }
    }

    /// The capabilities kept.
    pub fn kept(&self) -> &[Kept] {
        &self.kept
    }

    /// Where the capability `id` is, when kept.
    pub fn capability(&self, id: u8) -> Option<Kept> {
        self.kept.iter().find(|k| k.id == id).copied()
    }

    /// The shadow's 16 bits at `at`.
    pub fn shadow16(&self, at: usize) -> u16 {
        le16(&self.shadow, at)
    }

    /// The shadow's 32 bits at `at`.
    pub fn shadow32(&self, at: usize) -> u32 {
        le32(&self.shadow, at)
    }

    /// The command register as the device is to have it.
    pub fn device_command(&self) -> u16 {
        (self.shadow16(COMMAND) & COMMAND_TO_DEVICE) | COMMAND_INTX_OFF
    }

    /// The dword a BAR register reads as.
    fn bar_dword(&self, index: usize) -> u32 {
        let reg = self.bars[index];
        let Some(b) = reg.bar else {
            return 0;
        };
        let mask = !(b.size.max(16) - 1);
        let value = if reg.sizing { mask } else { b.address };
        if reg.high {
            #[allow(clippy::cast_possible_truncation)] // the upper half
            return (value >> 32) as u32;
        }
        let flags = if b.wide { 0b100 } else { 0 } | if b.prefetch { 0b1000 } else { 0 };
        #[allow(clippy::cast_possible_truncation)] // the lower half
        let low = value as u32;
        (low & !0xF) | flags
    }

    /// Whether `at` is a byte of a BAR register.
    fn in_bars(at: usize) -> bool {
        (BAR0..BAR_END).contains(&at)
    }

    /// One byte as the shadow and the kinds answer it; `live` is the
    /// device's byte there.
    fn byte(&self, at: usize, live: impl Fn(usize) -> u8) -> u8 {
        if at >= SPACE_BYTES {
            return 0;
        }
        if Self::in_bars(at) {
            let index = (at - BAR0) / 4;
            return self.bar_dword(index).to_le_bytes()[(at - BAR0) % 4];
        }
        match self.kind[at] {
            Kind::Hidden => 0,
            Kind::Shadow => self.shadow[at],
            Kind::Live => {
                let b = live(at);
                if at == STATUS {
                    // The capability list is the VMM's.
                    let caps = if self.kept.is_empty() { 0 } else { 1 << 4 };
                    (b & !(1 << 4)) | caps
                } else {
                    b
                }
            }
        }
    }

    /// Read `data.len()` bytes at `offset`; `live` reads the device's
    /// own byte. Anything past the space reads as zero.
    pub fn read(&self, offset: usize, data: &mut [u8], live: impl Fn(usize) -> u8) {
        for (i, d) in data.iter_mut().enumerate() {
            *d = offset.checked_add(i).map_or(0, |at| self.byte(at, &live));
        }
    }

    /// The guest writes `data` at `offset`: what changed.
    pub fn write(&mut self, offset: usize, data: &[u8]) -> Changed {
        let mut changed = Changed::default();
        let before_command = self.shadow16(COMMAND);
        let msi = self.capability(CAP_MSI);
        let msix = self.capability(CAP_MSIX);
        let before_msix = msix.map(|k| self.shadow16(k.at + 2));
        let mut status_clear = 0u16;
        let mut bar_dwords: [Option<[Option<u8>; 4]>; 6] = [None; 6];
        for (i, value) in data.iter().enumerate() {
            let Some(at) = offset.checked_add(i) else {
                break;
            };
            if at >= SPACE_BYTES {
                break;
            }
            if Self::in_bars(at) {
                let index = (at - BAR0) / 4;
                let d = bar_dwords[index].get_or_insert([None; 4]);
                d[(at - BAR0) % 4] = Some(*value);
                continue;
            }
            if (STATUS..STATUS + 2).contains(&at) {
                let shift = (at - STATUS) * 8;
                status_clear |= (u16::from(*value) << shift) & STATUS_RW1C;
                continue;
            }
            if self.kind[at] == Kind::Shadow {
                let w = self.writable[at];
                self.shadow[at] = (self.shadow[at] & !w) | (value & w);
            }
            if let Some(k) = msi
                && (k.at..k.at + k.len).contains(&at)
            {
                changed.msi = true;
            }
        }
        for (index, bytes) in bar_dwords.iter().enumerate() {
            let Some(bytes) = bytes else {
                continue;
            };
            let reg = &mut self.bars[index];
            // All ones asks the size (the first byte's flag bits aside);
            // any other value is an address the VMM does not take: the
            // BAR stays where it is.
            reg.sizing = reg.bar.is_some()
                && bytes.iter().enumerate().all(|(j, b)| match b {
                    Some(v) if j == 0 => v | 0xF == 0xFF,
                    Some(v) => *v == 0xFF,
                    None => true,
                });
        }
        if self.shadow16(COMMAND) != before_command {
            changed.command = Some(self.device_command());
        }
        if status_clear != 0 {
            changed.status = Some(status_clear);
        }
        if let (Some(k), Some(before)) = (msix, before_msix)
            && self.shadow16(k.at + 2) != before
        {
            changed.msix = Some(self.shadow16(k.at + 2));
        }
        changed
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A device's space: an NVMe-like function with power management,
    /// MSI (64-bit, masks), PCI Express, MSI-X, a vendor capability, a
    /// 64-bit BAR 0 and an I/O BAR 2, an expansion ROM and an `INTx` pin.
    pub(crate) fn device_space() -> Vec<u8> {
        let mut d = vec![0u8; SPACE_BYTES];
        d[0..4].copy_from_slice(&[0x36, 0x1b, 0x10, 0x00]);
        d[0x06] = 0x10;
        d[0x09..0x0C].copy_from_slice(&[0x02, 0x08, 0x01]);
        d[0x0E] = 0x80;
        d[0x10..0x14].copy_from_slice(&0xFEB0_0004u32.to_le_bytes());
        d[0x18..0x1C].copy_from_slice(&0x0000_C001u32.to_le_bytes());
        d[0x30..0x34].copy_from_slice(&0xFEA0_0000u32.to_le_bytes());
        d[0x34] = 0x40;
        d[0x3D] = 1;
        // PM at 0x40 -> MSI at 0x50 -> vendor at 0x70 -> PCIe at 0x80 -> MSI-X at 0xC0.
        d[0x40..0x48].copy_from_slice(&[CAP_PM, 0x50, 0x03, 0x00, 0x08, 0x00, 0x00, 0x00]);
        d[0x50] = CAP_MSI;
        d[0x51] = 0x70;
        d[0x52..0x54].copy_from_slice(&0x0186u16.to_le_bytes());
        d[0x70] = 0x09;
        d[0x71] = 0x80;
        d[0x72] = 0x10;
        d[0x80] = CAP_EXP;
        d[0x81] = 0xC0;
        d[0x82] = 0x02;
        d[0xC0] = CAP_MSIX;
        d[0xC1] = 0x00;
        d[0xC2..0xC4].copy_from_slice(&0x0040u16.to_le_bytes());
        d[0xC4..0xC8].copy_from_slice(&0x0000_2000u32.to_le_bytes());
        d[0xC8..0xCC].copy_from_slice(&0x0000_3000u32.to_le_bytes());
        d[0xF0] = 0xAB;
        d
    }

    pub(crate) fn bar0() -> Bar {
        Bar {
            index: 0,
            size: 0x4000,
            address: 0xE7FF_C000,
            wide: true,
            prefetch: false,
        }
    }

    fn read32(s: &Space, at: usize, dev: &[u8]) -> u32 {
        let mut b = [0u8; 4];
        s.read(at, &mut b, |a| dev[a]);
        u32::from_le_bytes(b)
    }

    #[test]
    fn the_header_is_the_vmm_s() {
        let dev = device_space();
        let s = Space::new(&dev, &[bar0()], 32);
        assert_eq!(read32(&s, 0, &dev), 0x0010_1b36);
        let mut h = [0u8; 1];
        s.read(0x0E, &mut h, |a| dev[a]);
        assert_eq!(h[0], 0, "the multifunction bit is cleared");
        // The I/O BAR, the ROM and the pin are not there.
        assert_eq!(read32(&s, 0x18, &dev), 0);
        assert_eq!(read32(&s, 0x30, &dev), 0);
        let mut pin = [0u8; 1];
        s.read(0x3D, &mut pin, |a| dev[a]);
        assert_eq!(pin[0], 0);
        // Device-specific bytes and extended space are hidden.
        assert_eq!(read32(&s, 0xF0, &dev), 0);
        assert_eq!(read32(&s, 0x100, &dev), 0);
        assert_eq!(read32(&s, 0xFFC, &dev), 0);
    }

    #[test]
    fn the_capabilities_kept_are_chained_without_the_rest() {
        let dev = device_space();
        let s = Space::new(&dev, &[bar0()], 32);
        let ids: Vec<(u8, usize)> = s.kept().iter().map(|k| (k.id, k.at)).collect();
        assert_eq!(
            ids,
            [
                (CAP_PM, 0x40),
                (CAP_MSI, 0x50),
                (CAP_EXP, 0x80),
                (CAP_MSIX, 0xC0)
            ]
        );
        let byte = |at| {
            let mut b = [0u8; 1];
            s.read(at, &mut b, |a| dev[a]);
            b[0]
        };
        assert_eq!(byte(0x34), 0x40);
        assert_eq!(byte(0x41), 0x50);
        assert_eq!(byte(0x51), 0x80, "the vendor capability is passed over");
        assert_eq!(byte(0x81), 0xC0);
        assert_eq!(byte(0xC1), 0);
        assert_eq!(byte(0x70), 0, "the vendor capability is hidden");
        assert_eq!(byte(0x06) & 0x10, 0x10);
        // The MSI-X table is the VMM's size.
        assert_eq!(s.shadow16(0xC2) & 0x7FF, 31);
    }

    #[test]
    fn a_bar_is_sized_and_never_moved() {
        let dev = device_space();
        let mut s = Space::new(&dev, &[bar0()], 32);
        assert_eq!(read32(&s, 0x10, &dev), 0xE7FF_C004);
        assert_eq!(read32(&s, 0x14, &dev), 0);
        s.write(0x10, &u32::MAX.to_le_bytes());
        s.write(0x14, &u32::MAX.to_le_bytes());
        assert_eq!(read32(&s, 0x10, &dev), 0xFFFF_C004);
        assert_eq!(read32(&s, 0x14, &dev), u32::MAX);
        s.write(0x10, &0x1234_0000u32.to_le_bytes());
        s.write(0x14, &0x0000_0001u32.to_le_bytes());
        assert_eq!(read32(&s, 0x10, &dev), 0xE7FF_C004);
        assert_eq!(read32(&s, 0x14, &dev), 0);
        // A byte at a time.
        for (i, b) in u32::MAX.to_le_bytes().iter().enumerate() {
            s.write(0x10 + i, &[*b]);
        }
        assert_eq!(read32(&s, 0x10, &dev), 0xFFFF_C004);
    }

    #[test]
    fn the_command_keeps_intx_off_and_io_off_at_the_device() {
        let dev = device_space();
        let mut s = Space::new(&dev, &[bar0()], 32);
        let c = s.write(0x04, &0xFFFFu16.to_le_bytes());
        assert_eq!(c.command, Some(COMMAND_TO_DEVICE | COMMAND_INTX_OFF));
        let c = s.write(0x04, &0u16.to_le_bytes());
        assert_eq!(c.command, Some(COMMAND_INTX_OFF));
        assert_eq!(s.write(0x04, &0u16.to_le_bytes()).command, None);
    }

    #[test]
    fn status_writes_clear_only_error_bits() {
        let dev = device_space();
        let mut s = Space::new(&dev, &[bar0()], 32);
        assert_eq!(
            s.write(0x06, &0xFFFFu16.to_le_bytes()).status,
            Some(STATUS_RW1C)
        );
        assert_eq!(s.write(0x06, &0x00FFu16.to_le_bytes()).status, None);
    }

    #[test]
    fn msi_and_msix_writes_are_seen_and_the_power_state_kept_here() {
        let dev = device_space();
        let mut s = Space::new(&dev, &[bar0()], 32);
        assert!(s.write(0x54, &[0, 0, 0xE0, 0xFE]).msi);
        let c = s.write(0xC2, &0xC000u16.to_le_bytes());
        assert_eq!(c.msix.map(|m| m & 0xC000), Some(0xC000));
        // The table size stays the VMM's.
        assert_eq!(s.shadow16(0xC2) & 0x7FF, 31);
        // D3 asked: kept here, the device untouched.
        let c = s.write(0x44, &[0x03]);
        assert_eq!(c, Changed::default());
        let mut b = [0u8; 1];
        s.read(0x44, &mut b, |a| dev[a]);
        assert_eq!(b[0] & 3, 3);
    }

    #[test]
    fn hostile_writes_change_nothing_outside_what_is_writable() {
        let dev = device_space();
        let mut s = Space::new(&dev, &[bar0()], 32);
        let before: Vec<u8> = (0..SPACE_BYTES)
            .map(|at| {
                let mut b = [0u8; 1];
                s.read(at, &mut b, |a| dev[a]);
                b[0]
            })
            .collect();
        for at in 0..SPACE_BYTES + 8 {
            s.write(at, &[0xFF; 8]);
        }
        s.write(usize::MAX - 2, &[0xFF; 8]);
        s.write(0, &[0xFF; 4096]);
        for at in [
            0usize, 1, 2, 3, 0x08, 0x09, 0x0A, 0x0B, 0x0E, 0x2C, 0x34, 0x3D, 0x41, 0x51, 0xC1,
            0xC4, 0xC8,
        ] {
            let mut b = [0u8; 1];
            s.read(at, &mut b, |a| dev[a]);
            assert_eq!(b[0], before[at], "byte {at:#x}");
        }
        assert_eq!(read32(&s, 0x30, &dev), 0);
        assert_eq!(read32(&s, 0x70, &dev), 0);
        assert_eq!(read32(&s, 0xF0, &dev), 0);
    }

    #[test]
    fn a_hostile_list_is_followed_a_bounded_time() {
        let mut dev = device_space();
        // A loop, a capability past the end, a pointer into the header.
        dev[0xC1] = 0x40;
        assert_eq!(kept_capabilities(&dev).len(), 4);
        dev[0x34] = 0xFC;
        dev[0xFC] = CAP_EXP;
        assert!(kept_capabilities(&dev).is_empty());
        dev[0x34] = 0x10;
        assert!(kept_capabilities(&dev).is_empty());
        assert!(kept_capabilities(&[]).is_empty());
    }
}
