// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! virtio-net: a partition's virtual Ethernet adapter. Its frames do
//! not pass through the VMM: the receive and transmit queues are served
//! by vhost-net in the kernel, from the tap of the adapter's port on its
//! switch, the guest's notifications reaching it by ioeventfd and its
//! interrupts raised by irqfd. The VMM serves the control queue alone
//! ([`ctrl`]).
//!
//! An adapter that came up without its switch (one that is not
//! required) has no tap: its link is down, and what the guest sends is
//! taken and goes nowhere.
//!
//! A reset by the guest stops vhost-net's queues and the next driver
//! starts them again. The partition's end closes the tap and vhost-net
//! with the process.

pub mod ctrl;

use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use virtio_queue::Queue;

use crate::virtio::worker::Worker;
use crate::virtio::{Activation, ActiveQueue, Interrupt, TYPE_NET, VirtioDevice};

/// `VIRTIO_NET_F_CSUM`: the guest's frames may leave their checksum.
pub const F_CSUM: u64 = 1 << 0;
/// `VIRTIO_NET_F_GUEST_CSUM`.
pub const F_GUEST_CSUM: u64 = 1 << 1;
/// `VIRTIO_NET_F_MTU`.
pub const F_MTU: u64 = 1 << 3;
/// `VIRTIO_NET_F_MAC`.
pub const F_MAC: u64 = 1 << 5;
/// `VIRTIO_NET_F_GUEST_TSO4`.
pub const F_GUEST_TSO4: u64 = 1 << 7;
/// `VIRTIO_NET_F_GUEST_TSO6`.
pub const F_GUEST_TSO6: u64 = 1 << 8;
/// `VIRTIO_NET_F_GUEST_ECN`.
pub const F_GUEST_ECN: u64 = 1 << 9;
/// `VIRTIO_NET_F_HOST_TSO4`.
pub const F_HOST_TSO4: u64 = 1 << 11;
/// `VIRTIO_NET_F_HOST_TSO6`.
pub const F_HOST_TSO6: u64 = 1 << 12;
/// `VIRTIO_NET_F_HOST_ECN`.
pub const F_HOST_ECN: u64 = 1 << 13;
/// `VIRTIO_NET_F_MRG_RXBUF`.
pub const F_MRG_RXBUF: u64 = 1 << 15;
/// `VIRTIO_NET_F_STATUS`.
pub const F_STATUS: u64 = 1 << 16;
/// `VIRTIO_NET_F_CTRL_VQ`.
pub const F_CTRL_VQ: u64 = 1 << 17;
/// `VIRTIO_NET_F_CTRL_RX`.
pub const F_CTRL_RX: u64 = 1 << 18;
/// `VIRTIO_NET_F_CTRL_MAC_ADDR`.
pub const F_CTRL_MAC_ADDR: u64 = 1 << 23;
/// `VIRTIO_NET_F_SPEED_DUPLEX`.
pub const F_SPEED_DUPLEX: u64 = 1 << 63;

/// The offloads, offered when there is a tap to carry them.
const OFFLOADS: u64 = F_CSUM
    | F_GUEST_CSUM
    | F_GUEST_TSO4
    | F_GUEST_TSO6
    | F_GUEST_ECN
    | F_HOST_TSO4
    | F_HOST_TSO6
    | F_HOST_ECN;

/// `VIRTIO_NET_S_LINK_UP`.
const LINK_UP: u16 = 1;
/// The speed an adapter says it has, in Mb/s.
const SPEED_MBPS: u32 = 10_000;
/// The receive and transmit queues' size.
const DATA_QUEUE: u16 = 256;
/// The control queue's size.
const CONTROL_QUEUE: u16 = 64;
/// The receive, transmit and control queues.
const RX: usize = 0;
const TX: usize = 1;
const CTRL: usize = 2;

/// The bytes of the configuration space: the address, the status, the
/// queue pairs, the MTU, the speed and the duplex.
pub const CONFIG_BYTES: usize = 17;

/// What the adapter is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Its MAC address, from the profile.
    pub mac: [u8; 6],
    /// The most the guest is to send: the switch's MTU.
    pub mtu: u16,
}

/// The kernel's side of a port: the tap and vhost-net.
#[derive(Debug)]
pub struct Backend {
    /// The tap of the port, from the switch service.
    pub tap: File,
    /// `/dev/vhost-net`, opened before the privilege drop.
    pub vhost: File,
}

/// What a guest's address change is told to.
pub type Said = Box<dyn FnMut([u8; 6]) + Send>;

/// The adapter.
pub struct Net {
    identity: Identity,
    backend: Option<Backend>,
    asked: Arc<Mutex<ctrl::Asked>>,
    said: Arc<Mutex<Said>>,
    worker: Option<Worker>,
    /// Whether vhost-net has its owner and the guest's memory.
    owned: bool,
    /// Whether vhost-net serves the queues now.
    running: bool,
}

impl Net {
    /// An adapter that is `identity`, over `backend`; without one its
    /// link is down. `said` is told each address the guest gives
    /// itself.
    pub fn new(identity: Identity, backend: Option<Backend>, said: Said) -> Self {
        Self {
            identity,
            backend,
            asked: Arc::new(Mutex::new(ctrl::Asked::default())),
            said: Arc::new(Mutex::new(said)),
            worker: None,
            owned: false,
            running: false,
        }
    }

    /// What the guest has asked for on the control queue.
    pub fn asked(&self) -> ctrl::Asked {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The configuration space's bytes.
    pub fn config_bytes(&self) -> [u8; CONFIG_BYTES] {
        let mut b = [0u8; CONFIG_BYTES];
        b[0..6].copy_from_slice(&self.identity.mac);
        let link = self.backend.is_some();
        b[6..8].copy_from_slice(&(if link { LINK_UP } else { 0 }).to_le_bytes());
        b[8..10].copy_from_slice(&1u16.to_le_bytes());
        b[10..12].copy_from_slice(&self.identity.mtu.to_le_bytes());
        let (speed, duplex) = if link {
            (SPEED_MBPS, 1)
        } else {
            (u32::MAX, 0xFF)
        };
        b[12..16].copy_from_slice(&speed.to_le_bytes());
        b[16] = duplex;
        b
    }

    fn stop(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.stop();
        }
        #[cfg(target_os = "linux")]
        if self.running
            && let Some(b) = &self.backend
        {
            for index in [RX, TX] {
                let index = u32::try_from(index).unwrap_or(0);
                let _ = crate::sys::vhost::set_backend(&b.vhost, index, None::<&File>);
                let _ = crate::sys::vhost::get_vring_base(&b.vhost, index);
            }
        }
        self.running = false;
    }

    /// The control queue on a worker of its own; with no backend, the
    /// transmit queue too, drained.
    fn serve(
        &mut self,
        control: Option<ActiveQueue>,
        drained: Option<ActiveQueue>,
        memory: Arc<crate::Memory>,
        interrupt: Interrupt,
    ) -> io::Result<()> {
        let mut notifies = Vec::new();
        let mut queues: Vec<(usize, Queue, u16)> = Vec::new();
        for (index, q) in [(CTRL, control), (TX, drained)] {
            if let Some(q) = q {
                notifies.push((index, q.notify));
                queues.push((index, q.queue, q.vector));
            }
        }
        if notifies.is_empty() {
            return Ok(());
        }
        let asked = self.asked.clone();
        let said = self.said.clone();
        self.worker = Some(Worker::spawn("virtio-net", notifies, move |index| {
            let Some((_, queue, vector)) = queues.iter_mut().find(|(i, ..)| *i == index) else {
                return;
            };
            let any = if index == CTRL {
                let mut asked = asked.lock().unwrap_or_else(PoisonError::into_inner);
                let mut said = said.lock().unwrap_or_else(PoisonError::into_inner);
                ctrl::serve(queue, &memory, &mut asked, &mut *said)
            } else {
                ctrl::drain(queue, &memory)
            };
            if any {
                interrupt.queue(*vector);
            }
        })?);
        Ok(())
    }
}

impl VirtioDevice for Net {
    fn device_type(&self) -> u32 {
        TYPE_NET
    }

    fn class(&self) -> u32 {
        // Network controller, Ethernet.
        0x02_00_00
    }

    fn features(&self) -> u64 {
        let base = F_MAC
            | F_MTU
            | F_STATUS
            | F_MRG_RXBUF
            | F_CTRL_VQ
            | F_CTRL_RX
            | F_CTRL_MAC_ADDR
            | F_SPEED_DUPLEX;
        if self.backend.is_some() {
            base | OFFLOADS
        } else {
            base
        }
    }

    fn queue_sizes(&self) -> Vec<u16> {
        vec![DATA_QUEUE, DATA_QUEUE, CONTROL_QUEUE]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let bytes = self.config_bytes();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = usize::try_from(offset)
                .ok()
                .and_then(|o| o.checked_add(i))
                .and_then(|at| bytes.get(at).copied())
                .unwrap_or(0);
        }
    }

    // The address is the profile's: a modern driver sets its own through
    // the control queue, where it is recorded.
    fn write_config(&mut self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, activation: Activation) -> io::Result<()> {
        let Activation {
            memory,
            mut queues,
            interrupt,
            features,
        } = activation;
        let take = |queues: &mut Vec<Option<ActiveQueue>>, i: usize| {
            queues.get_mut(i).and_then(Option::take)
        };
        let control = take(&mut queues, CTRL);
        let rx = take(&mut queues, RX);
        let tx = take(&mut queues, TX);
        if self.backend.is_none() {
            return self.serve(control, tx, memory, interrupt);
        }
        #[cfg(target_os = "linux")]
        {
            let (Some(rx), Some(tx)) = (rx, tx) else {
                return Err(io::Error::other(
                    "the receive and transmit queues are not enabled",
                ));
            };
            self.start(&memory, &interrupt, features, &[rx, tx])?;
            self.serve(control, None, memory, interrupt)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (rx, tx, features);
            Err(io::Error::other("vhost-net is Linux's"))
        }
    }

    fn reset(&mut self) {
        self.stop();
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(target_os = "linux")]
mod start;

#[cfg(test)]
mod tests {
    use super::*;

    fn said() -> Said {
        Box::new(|_| {})
    }

    #[test]
    fn an_adapter_says_its_address_link_and_mtu() {
        let identity = Identity {
            mac: [2, 0x5A, 0x3C, 0, 3, 5],
            mtu: 9000,
        };
        let down = Net::new(identity, None, said());
        let b = down.config_bytes();
        assert_eq!(&b[0..6], &identity.mac);
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 0);
        assert_eq!(u16::from_le_bytes([b[10], b[11]]), 9000);
        assert_eq!(&b[12..17], &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(down.features() & OFFLOADS, 0);
        assert_ne!(down.features() & F_CTRL_MAC_ADDR, 0);
        // Reads past the space are zeros, and do not panic.
        let mut data = [7u8; 4];
        down.read_config(15, &mut data);
        assert_eq!(data, [0xFF, 0xFF, 0, 0]);
        down.read_config(u64::MAX, &mut data);
        assert_eq!(data, [0; 4]);
    }
}
