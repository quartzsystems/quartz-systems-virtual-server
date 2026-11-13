// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Virtio over PCI, modern (version 1). A function has two memory
//! BARs:
//!
//! | BAR | Offset | What |
//! |---|---|---|
//! | 0 | `0000` | the common configuration |
//! | 0 | `1000` | the interrupt status byte |
//! | 0 | `2000` | the device's own configuration |
//! | 0 | `3000` | the notification area: four bytes a queue |
//! | 1 | `000` | the MSI-X table |
//! | 1 | `800` | the MSI-X pending bits |
//!
//! Interrupts are MSI-X only: the function has no interrupt pin. A
//! notification is an eventfd the kernel writes without leaving the
//! guest; one that does arrive as a memory write is passed on.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use virtio_queue::{Queue, QueueT};
use vmm_sys_util::eventfd::EventFd;

use crate::Memory;
use crate::pci::PciDevice;
use crate::pci::config::{Config, Identity};
use crate::pci::msix::Msix;
use crate::platform::Platform;
use crate::virtio::{
    Activation, ActiveQueue, FEATURE_INDIRECT_DESC, FEATURE_VERSION_1, Interrupt, NO_VECTOR,
    VirtioDevice,
};

/// The virtio vendor.
const VENDOR: u16 = 0x1AF4;
/// Modern device ids start here; the device type is added.
const DEVICE_BASE: u16 = 0x1040;

const BAR0_BYTES: u32 = 0x4000;
const BAR1_BYTES: u32 = 0x1000;
const COMMON_OFFSET: u64 = 0x0000;
const COMMON_LEN: usize = 0x38;
const COMMON_BYTES: u64 = COMMON_LEN as u64;
const ISR_OFFSET: u64 = 0x1000;
const DEVICE_OFFSET: u64 = 0x2000;
const DEVICE_BYTES: u64 = 0x1000;
const NOTIFY_OFFSET: u64 = 0x3000;
const NOTIFY_MULTIPLIER: u32 = 4;
const PENDING_OFFSET: u64 = 0x800;

/// The vendor-specific capability.
const CAP_VENDOR: u8 = 0x09;
/// The MSI-X capability.
const CAP_MSIX: u8 = 0x11;
const CFG_COMMON: u8 = 1;
const CFG_NOTIFY: u8 = 2;
const CFG_ISR: u8 = 3;
const CFG_DEVICE: u8 = 4;

const STATUS_FEATURES_OK: u8 = 0x08;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_NEEDS_RESET: u8 = 0x40;

/// The most queues a device here has.
pub const MAX_QUEUES: usize = 16;

#[derive(Clone, Copy)]
struct QueueConfig {
    max: u16,
    size: u16,
    vector: u16,
    enabled: bool,
    desc: u64,
    avail: u64,
    used: u64,
}

impl QueueConfig {
    fn new(max: u16) -> Self {
        Self {
            max,
            size: max,
            vector: NO_VECTOR,
            enabled: false,
            desc: 0,
            avail: 0,
            used: 0,
        }
    }
}

/// One virtio function.
pub struct VirtioPci {
    config: Config,
    msix_capability: usize,
    msix: Arc<Mutex<Msix>>,
    device: Box<dyn VirtioDevice>,
    platform: Arc<dyn Platform>,
    memory: Arc<Memory>,
    isr: Arc<AtomicU8>,
    queues: Vec<QueueConfig>,
    notifies: Vec<EventFd>,
    /// Where the notification eventfds are registered now.
    registered: Option<u64>,
    device_feature_select: u32,
    driver_feature_select: u32,
    driver_features: u64,
    config_vector: u16,
    status: u8,
    queue_select: u16,
    active: bool,
}

/// A vendor capability's body: where a structure is.
fn vendor_capability(kind: u8, offset: u64, bytes: u64, multiplier: Option<u32>) -> Vec<u8> {
    let length: u8 = if multiplier.is_some() { 20 } else { 16 };
    let mut body = vec![length, kind, 0, 0, 0, 0];
    #[allow(clippy::cast_possible_truncation)] // offsets within a 16 KiB BAR
    {
        body.extend_from_slice(&(offset as u32).to_le_bytes());
        body.extend_from_slice(&(bytes as u32).to_le_bytes());
    }
    if let Some(multiplier) = multiplier {
        body.extend_from_slice(&multiplier.to_le_bytes());
    }
    body
}

impl VirtioPci {
    /// The function for `device`, with its BARs at `bar0` and `bar1`.
    pub fn new(
        device: Box<dyn VirtioDevice>,
        platform: Arc<dyn Platform>,
        memory: Arc<Memory>,
        bar0: u32,
        bar1: u32,
    ) -> io::Result<Self> {
        let sizes = device.queue_sizes();
        if sizes.is_empty() || sizes.len() > MAX_QUEUES {
            return Err(io::Error::other("queue count"));
        }
        let kind = u16::try_from(device.device_type()).map_err(io::Error::other)?;
        let mut config = Config::new(&Identity {
            vendor: VENDOR,
            device: DEVICE_BASE + kind,
            revision: 1,
            class: device.class(),
            subsystem_vendor: VENDOR,
            subsystem: kind,
        });
        if !config.add_bar(0, bar0, BAR0_BYTES) || !config.add_bar(1, bar1, BAR1_BYTES) {
            return Err(io::Error::other("BAR address"));
        }
        let notify_bytes = sizes.len() as u64 * u64::from(NOTIFY_MULTIPLIER);
        let capabilities = [
            vendor_capability(CFG_COMMON, COMMON_OFFSET, COMMON_BYTES, None),
            vendor_capability(
                CFG_NOTIFY,
                NOTIFY_OFFSET,
                notify_bytes,
                Some(NOTIFY_MULTIPLIER),
            ),
            vendor_capability(CFG_ISR, ISR_OFFSET, 1, None),
            vendor_capability(CFG_DEVICE, DEVICE_OFFSET, DEVICE_BYTES, None),
        ];
        for body in &capabilities {
            config
                .add_capability(CAP_VENDOR, body, &[])
                .ok_or_else(|| io::Error::other("capabilities"))?;
        }
        // One vector for configuration changes and one a queue.
        let vectors = sizes.len() + 1;
        let msix = Msix::new(platform.clone(), vectors)?;
        let mut body = Vec::with_capacity(10);
        #[allow(clippy::cast_possible_truncation)] // 17 vectors at most
        body.extend_from_slice(&(vectors as u16 - 1).to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes()); // the table: BAR 1, offset 0
        #[allow(clippy::cast_possible_truncation)]
        body.extend_from_slice(&(PENDING_OFFSET as u32 | 1).to_le_bytes());
        let msix_capability = config
            .add_capability(CAP_MSIX, &body, &[0x00, 0xC0])
            .ok_or_else(|| io::Error::other("capabilities"))?;
        let mut notifies = Vec::with_capacity(sizes.len());
        for _ in &sizes {
            notifies.push(EventFd::new(0)?);
        }
        Ok(Self {
            config,
            msix_capability,
            msix: Arc::new(Mutex::new(msix)),
            device,
            platform,
            memory,
            isr: Arc::new(AtomicU8::new(0)),
            queues: sizes.into_iter().map(QueueConfig::new).collect(),
            notifies,
            registered: None,
            device_feature_select: 0,
            driver_feature_select: 0,
            driver_features: 0,
            config_vector: NO_VECTOR,
            status: 0,
            queue_select: 0,
            active: false,
        })
    }

    fn device_features(&self) -> u64 {
        self.device.features() | FEATURE_VERSION_1 | FEATURE_INDIRECT_DESC
    }

    /// Keep the notification eventfds where the notification area is.
    fn place_notifications(&mut self) {
        let now = self.config.bar(0).map(|(start, _)| start + NOTIFY_OFFSET);
        if now == self.registered {
            return;
        }
        let step = u64::from(NOTIFY_MULTIPLIER);
        if let Some(old) = self.registered.take() {
            for (fd, i) in self.notifies.iter().zip(0u64..) {
                let _ = self.platform.remove_ioeventfd(old + i * step, fd);
            }
        }
        if let Some(new) = now {
            let done = self
                .notifies
                .iter()
                .zip(0u64..)
                .all(|(fd, i)| self.platform.ioeventfd(new + i * step, fd).is_ok());
            // Unregistered, a notification still arrives as a write.
            self.registered = done.then_some(new);
        }
    }

    fn selected(&mut self) -> Option<&mut QueueConfig> {
        self.queues.get_mut(usize::from(self.queue_select))
    }

    fn reset(&mut self) {
        self.device.reset();
        self.active = false;
        for q in &mut self.queues {
            *q = QueueConfig::new(q.max);
        }
        self.device_feature_select = 0;
        self.driver_feature_select = 0;
        self.driver_features = 0;
        self.config_vector = NO_VECTOR;
        self.status = 0;
        self.queue_select = 0;
        self.isr.store(0, Ordering::SeqCst);
    }

    /// The queues the guest enabled, as the device takes them. `None`
    /// when one of them is not a queue.
    fn build_queues(&self) -> Option<Vec<Option<ActiveQueue>>> {
        let mut out = Vec::with_capacity(self.queues.len());
        for (config, notify) in self.queues.iter().zip(&self.notifies) {
            if !config.enabled {
                out.push(None);
                continue;
            }
            let mut queue = Queue::new(config.max).ok()?;
            queue.set_size(config.size);
            #[allow(clippy::cast_possible_truncation)] // the two halves
            let halves = |v: u64| (Some(v as u32), Some((v >> 32) as u32));
            let (low, high) = halves(config.desc);
            queue.set_desc_table_address(low, high);
            let (low, high) = halves(config.avail);
            queue.set_avail_ring_address(low, high);
            let (low, high) = halves(config.used);
            queue.set_used_ring_address(low, high);
            queue.set_ready(true);
            if !queue.is_valid(self.memory.as_ref()) {
                return None;
            }
            out.push(Some(ActiveQueue {
                queue,
                notify: notify.try_clone().ok()?,
                vector: config.vector,
            }));
        }
        Some(out)
    }

    fn activate(&mut self) {
        let activation = self.build_queues().map(|queues| Activation {
            memory: self.memory.clone(),
            queues,
            interrupt: Interrupt::new(self.msix.clone(), self.isr.clone()),
            features: self.driver_features,
        });
        match activation.map(|a| self.device.activate(a)) {
            Some(Ok(())) => self.active = true,
            _ => self.status |= STATUS_NEEDS_RESET,
        }
    }

    fn set_status(&mut self, value: u8) {
        if value == 0 {
            self.reset();
            return;
        }
        let mut value = value;
        let features_ok = self.driver_features & !self.device_features() == 0
            && self.driver_features & FEATURE_VERSION_1 != 0;
        if value & STATUS_FEATURES_OK != 0 && !features_ok {
            value &= !STATUS_FEATURES_OK;
        }
        // A bit once set stays until a reset.
        self.status |= value & !STATUS_NEEDS_RESET;
        if self.status & STATUS_DRIVER_OK != 0
            && self.status & STATUS_FEATURES_OK != 0
            && !self.active
            && self.status & STATUS_NEEDS_RESET == 0
        {
            self.activate();
        }
    }

    /// The common configuration as its 56 bytes.
    fn common(&self) -> [u8; COMMON_LEN] {
        let mut b = [0u8; COMMON_LEN];
        let features = self.device_features();
        #[allow(clippy::cast_possible_truncation)] // the selected half
        let device_feature = match self.device_feature_select {
            0 => features as u32,
            1 => (features >> 32) as u32,
            _ => 0,
        };
        #[allow(clippy::cast_possible_truncation)]
        let driver_feature = match self.driver_feature_select {
            0 => self.driver_features as u32,
            1 => (self.driver_features >> 32) as u32,
            _ => 0,
        };
        b[0x00..0x04].copy_from_slice(&self.device_feature_select.to_le_bytes());
        b[0x04..0x08].copy_from_slice(&device_feature.to_le_bytes());
        b[0x08..0x0C].copy_from_slice(&self.driver_feature_select.to_le_bytes());
        b[0x0C..0x10].copy_from_slice(&driver_feature.to_le_bytes());
        b[0x10..0x12].copy_from_slice(&self.config_vector.to_le_bytes());
        #[allow(clippy::cast_possible_truncation)] // sixteen at most
        b[0x12..0x14].copy_from_slice(&(self.queues.len() as u16).to_le_bytes());
        b[0x14] = self.status;
        b[0x16..0x18].copy_from_slice(&self.queue_select.to_le_bytes());
        if let Some(q) = self.queues.get(usize::from(self.queue_select)) {
            b[0x18..0x1A].copy_from_slice(&q.size.to_le_bytes());
            b[0x1A..0x1C].copy_from_slice(&q.vector.to_le_bytes());
            b[0x1C..0x1E].copy_from_slice(&u16::from(q.enabled).to_le_bytes());
            b[0x1E..0x20].copy_from_slice(&self.queue_select.to_le_bytes());
            b[0x20..0x28].copy_from_slice(&q.desc.to_le_bytes());
            b[0x28..0x30].copy_from_slice(&q.avail.to_le_bytes());
            b[0x30..0x38].copy_from_slice(&q.used.to_le_bytes());
        }
        b
    }

    fn write_common(&mut self, offset: u64, data: &[u8]) {
        let u16_of = |d: &[u8]| (d.len() == 2).then(|| u16::from_le_bytes([d[0], d[1]]));
        let u32_of =
            |d: &[u8]| (d.len() == 4).then(|| u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
        let vectors = self.msix.lock().map_or(0, |m| m.len());
        // A vector the table does not have reads back as "none".
        let vector = |v: u16| {
            if usize::from(v) < vectors {
                v
            } else {
                NO_VECTOR
            }
        };
        // The queue's addresses and size are fixed once it is live.
        let live = self.active;
        match offset {
            0x00 => {
                if let Some(v) = u32_of(data) {
                    self.device_feature_select = v;
                }
            }
            0x08 => {
                if let Some(v) = u32_of(data) {
                    self.driver_feature_select = v;
                }
            }
            0x0C => {
                if let Some(v) = u32_of(data)
                    && self.status & STATUS_FEATURES_OK == 0
                {
                    match self.driver_feature_select {
                        0 => {
                            self.driver_features =
                                (self.driver_features & !0xFFFF_FFFF) | u64::from(v);
                        }
                        1 => {
                            self.driver_features =
                                (self.driver_features & 0xFFFF_FFFF) | (u64::from(v) << 32);
                        }
                        _ => {}
                    }
                }
            }
            0x10 => {
                if let Some(v) = u16_of(data) {
                    self.config_vector = vector(v);
                }
            }
            0x14 => {
                if let Some(v) = data.first() {
                    self.set_status(*v);
                }
            }
            0x16 => {
                if let Some(v) = u16_of(data) {
                    self.queue_select = v;
                }
            }
            0x18 => {
                if let (Some(v), Some(q)) = (u16_of(data), self.selected())
                    && !live
                    && v != 0
                    && v.is_power_of_two()
                    && v <= q.max
                {
                    q.size = v;
                }
            }
            0x1A => {
                if let (Some(v), Some(q)) = (u16_of(data), self.selected()) {
                    q.vector = vector(v);
                }
            }
            0x1C => {
                if let (Some(v), Some(q)) = (u16_of(data), self.selected())
                    && !live
                {
                    // A queue is enabled once; only a reset disables it.
                    q.enabled |= v == 1;
                }
            }
            0x20..0x38 => {
                let Some(v) = u32_of(data) else { return };
                let Some(q) = self.selected() else { return };
                if live || q.enabled {
                    return;
                }
                let field = match offset & !7 {
                    0x20 => &mut q.desc,
                    0x28 => &mut q.avail,
                    _ => &mut q.used,
                };
                *field = if offset & 4 == 0 {
                    (*field & !0xFFFF_FFFF) | u64::from(v)
                } else {
                    (*field & 0xFFFF_FFFF) | (u64::from(v) << 32)
                };
            }
            _ => {}
        }
    }
}

impl PciDevice for VirtioPci {
    fn read_config(&mut self, offset: usize, data: &mut [u8]) {
        self.config.read(offset, data);
    }

    fn write_config(&mut self, offset: usize, data: &[u8]) {
        self.config.write(offset, data);
        let control = u16::from_le_bytes([
            self.config.byte(self.msix_capability + 2),
            self.config.byte(self.msix_capability + 3),
        ]);
        if let Ok(mut msix) = self.msix.lock() {
            msix.set_control(control);
        }
        self.place_notifications();
    }

    fn bar_at(&self, address: u64) -> Option<(usize, u64)> {
        self.config.bar_at(address)
    }

    fn read_bar(&mut self, bar: usize, offset: u64, data: &mut [u8]) {
        data.fill(0xFF);
        match (bar, offset) {
            (0, o) if o < COMMON_BYTES => {
                let common = self.common();
                for (i, byte) in data.iter_mut().enumerate() {
                    #[allow(clippy::cast_possible_truncation)] // below 56
                    if let Some(b) = common.get(o as usize + i) {
                        *byte = *b;
                    }
                }
            }
            (0, ISR_OFFSET) => {
                // Reading the status clears it.
                if let Some(first) = data.first_mut() {
                    *first = self.isr.swap(0, Ordering::SeqCst);
                }
            }
            (0, o) if (DEVICE_OFFSET..DEVICE_OFFSET + DEVICE_BYTES).contains(&o) => {
                self.device.read_config(o - DEVICE_OFFSET, data);
            }
            (1, o) if o < PENDING_OFFSET => {
                if let Ok(msix) = self.msix.lock() {
                    msix.read_table(o, data);
                }
            }
            (1, o) => {
                if let Ok(msix) = self.msix.lock() {
                    msix.read_pending(o - PENDING_OFFSET, data);
                }
            }
            _ => {}
        }
    }

    fn write_bar(&mut self, bar: usize, offset: u64, data: &[u8]) {
        match (bar, offset) {
            (0, o) if o < COMMON_BYTES => self.write_common(o, data),
            (0, o) if (DEVICE_OFFSET..DEVICE_OFFSET + DEVICE_BYTES).contains(&o) => {
                self.device.write_config(o - DEVICE_OFFSET, data);
            }
            (0, o) if o >= NOTIFY_OFFSET => {
                let queue = (o - NOTIFY_OFFSET) / u64::from(NOTIFY_MULTIPLIER);
                if let Some(fd) = usize::try_from(queue)
                    .ok()
                    .and_then(|q| self.notifies.get(q))
                {
                    let _ = fd.write(1);
                }
            }
            (1, o) if o < PENDING_OFFSET => {
                if let Ok(mut msix) = self.msix.lock() {
                    msix.write_table(o, data);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::fake::Recorder;
    use std::sync::mpsc;
    use vm_memory::GuestAddress;

    struct Probe {
        events: mpsc::Sender<&'static str>,
        activation: Option<Activation>,
    }

    impl VirtioDevice for Probe {
        fn device_type(&self) -> u32 {
            crate::virtio::TYPE_RNG
        }
        fn class(&self) -> u32 {
            0x00_FF_00
        }
        fn features(&self) -> u64 {
            1
        }
        fn queue_sizes(&self) -> Vec<u16> {
            vec![64, 64]
        }
        fn read_config(&self, offset: u64, data: &mut [u8]) {
            #[allow(clippy::cast_possible_truncation)]
            data.fill(offset as u8);
        }
        fn write_config(&mut self, _: u64, _: &[u8]) {}
        fn activate(&mut self, activation: Activation) -> io::Result<()> {
            self.activation = Some(activation);
            self.events.send("activate").unwrap();
            Ok(())
        }
        fn reset(&mut self) {
            self.activation = None;
            self.events.send("reset").unwrap();
        }
    }

    const BAR0: u64 = 0xC000_0000;

    fn function() -> (VirtioPci, mpsc::Receiver<&'static str>, Arc<Recorder>) {
        let (events, seen) = mpsc::channel();
        let platform = Arc::new(Recorder::default());
        let memory = Arc::new(Memory::from_ranges(&[(GuestAddress(0), 0x10_0000)]).unwrap());
        let probe = Probe {
            events,
            activation: None,
        };
        #[allow(clippy::cast_possible_truncation)]
        let f = VirtioPci::new(
            Box::new(probe),
            platform.clone(),
            memory,
            BAR0 as u32,
            0xC000_4000,
        )
        .unwrap();
        (f, seen, platform)
    }

    fn read32(f: &mut VirtioPci, offset: u64) -> u32 {
        let mut b = [0u8; 4];
        f.read_bar(0, offset, &mut b);
        u32::from_le_bytes(b)
    }

    fn read16(f: &mut VirtioPci, offset: u64) -> u16 {
        let mut b = [0u8; 2];
        f.read_bar(0, offset, &mut b);
        u16::from_le_bytes(b)
    }

    /// What a driver does up to `DRIVER_OK`, with queue 0 at `desc`.
    fn bring_up(f: &mut VirtioPci, desc: u64) {
        f.write_bar(0, 0x14, &[1]);
        f.write_bar(0, 0x14, &[3]);
        f.write_bar(0, 0x08, &1u32.to_le_bytes());
        f.write_bar(0, 0x0C, &1u32.to_le_bytes()); // VERSION_1
        f.write_bar(0, 0x14, &[0x0B]);
        f.write_bar(0, 0x16, &0u16.to_le_bytes());
        f.write_bar(0, 0x18, &16u16.to_le_bytes());
        f.write_bar(0, 0x1A, &1u16.to_le_bytes());
        #[allow(clippy::cast_possible_truncation)]
        for (at, value) in [(0x20, desc), (0x28, desc + 0x1000), (0x30, desc + 0x2000)] {
            f.write_bar(0, at, &(value as u32).to_le_bytes());
            f.write_bar(0, at + 4, &((value >> 32) as u32).to_le_bytes());
        }
        f.write_bar(0, 0x1C, &1u16.to_le_bytes());
        f.write_bar(0, 0x14, &[0x0F]);
    }

    #[test]
    fn the_capabilities_say_where_each_structure_is() {
        let (mut f, _, _) = function();
        let mut id = [0u8; 4];
        f.read_config(0, &mut id);
        assert_eq!(u32::from_le_bytes(id), 0x1044_1AF4);
        // Walk the list: four vendor capabilities, then MSI-X.
        let mut at = usize::from(f.config.byte(0x34));
        let mut seen = Vec::new();
        while at != 0 {
            seen.push((f.config.byte(at), f.config.byte(at + 3)));
            at = usize::from(f.config.byte(at + 1));
        }
        assert_eq!(seen[..4], [(9, 1), (9, 2), (9, 3), (9, 4)]);
        assert_eq!(seen[4].0, 0x11);
        // Three vectors: configuration and two queues.
        assert_eq!(f.config.byte(f.msix_capability + 2), 2);
        // No interrupt pin.
        assert_eq!(f.config.byte(0x3D), 0);
    }

    #[test]
    fn a_driver_s_bring_up_activates_the_device_once() {
        let (mut f, events, _) = function();
        assert_eq!(read32(&mut f, 0x04), 1 | (1 << 28));
        f.write_bar(0, 0x00, &1u32.to_le_bytes());
        assert_eq!(read32(&mut f, 0x04), 1);
        assert_eq!(read16(&mut f, 0x12), 2);
        bring_up(&mut f, 0x1_0000);
        assert_eq!(events.try_recv(), Ok("activate"));
        let mut status = [0u8];
        f.read_bar(0, 0x14, &mut status);
        assert_eq!(status, [0x0F]);
        // A second DRIVER_OK does nothing; the live queue cannot move.
        f.write_bar(0, 0x14, &[0x0F]);
        f.write_bar(0, 0x20, &0u32.to_le_bytes());
        assert!(events.try_recv().is_err());
        assert_eq!(read32(&mut f, 0x20), 0x1_0000);
        assert_eq!(read16(&mut f, 0x1A), 1);
        // The reset.
        f.write_bar(0, 0x14, &[0]);
        assert_eq!(events.try_recv(), Ok("reset"));
        assert_eq!(read32(&mut f, 0x20), 0);
        assert_eq!(read16(&mut f, 0x1A), NO_VECTOR);
    }

    #[test]
    fn a_queue_outside_memory_needs_a_reset_and_is_not_served() {
        let (mut f, events, _) = function();
        bring_up(&mut f, 0xFFFF_FFFF_0000);
        assert!(events.try_recv().is_err());
        let mut status = [0u8];
        f.read_bar(0, 0x14, &mut status);
        assert_ne!(status[0] & STATUS_NEEDS_RESET, 0);
    }

    #[test]
    fn features_the_device_lacks_are_refused() {
        let (mut f, events, _) = function();
        f.write_bar(0, 0x14, &[3]);
        f.write_bar(0, 0x08, &0u32.to_le_bytes());
        f.write_bar(0, 0x0C, &0xFFFF_FFFFu32.to_le_bytes());
        f.write_bar(0, 0x08, &1u32.to_le_bytes());
        f.write_bar(0, 0x0C, &1u32.to_le_bytes());
        f.write_bar(0, 0x14, &[0x0B]);
        let mut status = [0u8];
        f.read_bar(0, 0x14, &mut status);
        assert_eq!(status[0] & STATUS_FEATURES_OK, 0);
        f.write_bar(0, 0x14, &[0x0F]);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn notifications_follow_the_bar_and_odd_writes_are_harmless() {
        let (mut f, _, platform) = function();
        assert!(platform.ioevents.lock().unwrap().is_empty());
        f.write_config(4, &2u16.to_le_bytes());
        assert_eq!(
            *platform.ioevents.lock().unwrap(),
            [BAR0 + 0x3000, BAR0 + 0x3004]
        );
        f.write_config(0x10, &0xD000_0000u32.to_le_bytes());
        assert_eq!(
            *platform.ioevents.lock().unwrap(),
            [0xD000_3000, 0xD000_3004]
        );
        // A notification that arrives as a write is passed on.
        f.write_bar(0, 0x3004, &1u16.to_le_bytes());
        assert_eq!(f.notifies[1].read().unwrap(), 1);
        // Nothing here may panic.
        f.write_bar(0, 0x3FFC, &[0; 4]);
        f.write_bar(0, 0x18, &[1, 2, 3]);
        f.write_bar(0, 0x16, &0xFFFFu16.to_le_bytes());
        f.write_bar(0, 0x18, &16u16.to_le_bytes());
        f.write_bar(7, 0, &[0; 4]);
        f.write_bar(1, 0xFFF, &[0; 8]);
        let mut data = [0u8; 8];
        f.read_bar(0, 0x34, &mut data);
        f.read_bar(1, 0xFFC, &mut data);
        f.read_bar(0, u64::MAX, &mut data);
        f.read_bar(0, 0x2FFF, &mut data);
        assert_eq!(read16(&mut f, 0x18), 0);
    }
}
