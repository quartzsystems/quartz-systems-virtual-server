// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The virtio devices of a partition.
//!
//! - [`transport`]: virtio over PCI, modern (version 1) only: the
//!   capabilities, the common configuration, MSI-X, notification by
//!   eventfd.
//! - [`worker`]: the thread a device's queues are served on, off the
//!   processors' threads.
//! - [`scsi`]: the SCSI host, with disk LUNs.
//! - [`rng`]: the entropy source.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use virtio_queue::Queue;
use vmm_sys_util::eventfd::EventFd;

use crate::Memory;
use crate::pci::Msix;

pub mod rng;
pub mod scsi;
pub mod transport;
pub mod worker;

pub use transport::VirtioPci;

/// `VIRTIO_F_VERSION_1`.
pub const FEATURE_VERSION_1: u64 = 1 << 32;
/// `VIRTIO_F_RING_INDIRECT_DESC`.
pub const FEATURE_INDIRECT_DESC: u64 = 1 << 28;

/// The virtio device type of an entropy source.
pub const TYPE_RNG: u32 = 4;
/// The virtio device type of a SCSI host.
pub const TYPE_SCSI: u32 = 8;

/// The guest's "no vector".
pub const NO_VECTOR: u16 = 0xFFFF;

/// How a device interrupts the guest.
#[derive(Clone)]
pub struct Interrupt {
    msix: Arc<Mutex<Msix>>,
    isr: Arc<AtomicU8>,
}

impl Interrupt {
    /// An interrupt over `msix`, with the status byte `isr`.
    pub fn new(msix: Arc<Mutex<Msix>>, isr: Arc<AtomicU8>) -> Self {
        Self { msix, isr }
    }

    /// A queue has used buffers: raise its vector.
    pub fn queue(&self, vector: u16) {
        self.isr.fetch_or(1, Ordering::SeqCst);
        if let Ok(mut msix) = self.msix.lock() {
            msix.signal(vector);
        }
    }
}

/// One queue the guest made ready.
pub struct ActiveQueue {
    /// The queue.
    pub queue: Queue,
    /// Written when the guest has put buffers in it.
    pub notify: EventFd,
    /// The vector the guest wants for it.
    pub vector: u16,
}

/// What a device is given when the guest's driver is ready.
pub struct Activation {
    /// The guest's memory.
    pub memory: Arc<Memory>,
    /// The device's queues by number; `None` for one the guest left
    /// disabled.
    pub queues: Vec<Option<ActiveQueue>>,
    /// The way to interrupt.
    pub interrupt: Interrupt,
    /// The features the driver accepted.
    pub features: u64,
}

/// A virtio device, under the transport.
pub trait VirtioDevice: Send {
    /// The virtio device type.
    fn device_type(&self) -> u32;
    /// The PCI class, subclass and programming interface.
    fn class(&self) -> u32;
    /// The device's own feature bits; the transport adds its own.
    fn features(&self) -> u64;
    /// The largest size of each queue.
    fn queue_sizes(&self) -> Vec<u16>;
    /// Read the device configuration.
    fn read_config(&self, offset: u64, data: &mut [u8]);
    /// Write the device configuration.
    fn write_config(&mut self, offset: u64, data: &[u8]);
    /// The driver is ready: serve the queues.
    fn activate(&mut self, activation: Activation) -> io::Result<()>;
    /// The driver reset the device: stop serving, forget everything.
    fn reset(&mut self);
}
