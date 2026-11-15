// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! virtio-console with several ports: the partition's service channel
//! and its console port (`docs/VMM.md`). The ports have IDs from 1 and
//! are announced by name on the control queue; port 0, which a guest
//! takes for a console of its own, is not used.
//!
//! | Queue | What |
//! |---|---|
//! | 0, 1 | port 0's, not used |
//! | 2, 3 | control: to the guest, from the guest |
//! | 2(n+1), 2(n+1)+1 | port n: to the guest, from the guest |
//!
//! Each port is joined to the host by a [`Link`]. The device carries
//! bytes and reads none of them: what the ports carry is the business
//! of whoever holds the link's other end. Every descriptor is the
//! guest's: a chain is read and written within the bounds it gives, and
//! a buffer larger than a frame is read only to [`MAX_CHAIN`].

pub mod control;
pub mod link;

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, PoisonError};

use virtio_queue::{Queue, QueueT};

use crate::Memory;
use crate::virtio::worker::Worker;
use crate::virtio::{Activation, ActiveQueue, Interrupt, TYPE_CONSOLE, VirtioDevice};
pub use control::Control;
pub use link::Link;

/// `VIRTIO_CONSOLE_F_MULTIPORT`.
pub const FEATURE_MULTIPORT: u64 = 1 << 1;
/// Each queue's size.
const QUEUE_SIZE: u16 = 64;
/// The most bytes read of one buffer the guest sends: two frames.
pub const MAX_CHAIN: usize = 64 * 1024;
/// The control queues.
const CONTROL_RX: usize = 2;
const CONTROL_TX: usize = 3;

/// The queue that carries port `index`'s bytes to the guest.
pub fn rx_queue(index: usize) -> usize {
    2 * (index + 2)
}

/// The queue that carries them from the guest.
pub fn tx_queue(index: usize) -> usize {
    rx_queue(index) + 1
}

/// A named port and its link.
#[derive(Clone)]
pub struct Port {
    /// The name the guest finds it by.
    pub name: String,
    /// Its link to the host.
    pub link: Arc<Link>,
}

/// The device.
pub struct Console {
    ports: Vec<Port>,
    control: Arc<Mutex<Control>>,
    worker: Option<Worker>,
}

impl Console {
    /// A device with `ports`, IDs from 1 in their order.
    pub fn new(ports: Vec<Port>) -> Self {
        let names: Vec<&str> = ports.iter().map(|p| p.name.as_str()).collect();
        let control = Arc::new(Mutex::new(Control::new(&names)));
        Self {
            ports,
            control,
            worker: None,
        }
    }

    fn queue_count(&self) -> usize {
        rx_queue(self.ports.len())
    }
}

/// Take what the guest sent on the control queue. Whether a buffer was
/// used.
pub fn serve_control_tx(
    queue: &mut Queue,
    memory: &Memory,
    control: &mut Control,
    open: &dyn Fn(usize) -> bool,
) -> bool {
    let mut any = false;
    while let Some(chain) = queue.pop_descriptor_chain(memory) {
        let head = chain.head_index();
        let mut bytes = [0u8; control::HEADER];
        let mut got = 0;
        if let Ok(mut reader) = chain.reader(memory) {
            got = reader.read(&mut bytes).unwrap_or(0);
        }
        control.from_guest(&bytes[..got], open);
        if queue.add_used(memory, head, 0).is_err() {
            break;
        }
        any = true;
    }
    any
}

/// Give the guest the control messages that wait, as far as it has
/// buffers. Whether one was used.
pub fn fill_control_rx(queue: &mut Queue, memory: &Memory, control: &mut Control) -> bool {
    let mut any = false;
    while control.pending() {
        let Some(chain) = queue.pop_descriptor_chain(memory) else {
            break;
        };
        let head = chain.head_index();
        let Some(message) = control.take() else {
            break;
        };
        let mut given = 0;
        if let Ok(mut writer) = chain.writer(memory) {
            let room = writer.available_bytes().min(message.len());
            if room >= control::HEADER && writer.write_all(&message[..room]).is_ok() {
                given = room;
            }
        }
        #[allow(clippy::cast_possible_truncation)] // a name's bytes
        if queue.add_used(memory, head, given as u32).is_err() {
            break;
        }
        any = true;
    }
    any
}

/// Take what the guest sent on a port, while the link takes it.
pub fn serve_tx(queue: &mut Queue, memory: &Memory, link: &Link) -> bool {
    let mut any = false;
    let mut bytes = vec![0u8; MAX_CHAIN];
    while link.takes_from_guest() {
        let Some(chain) = queue.pop_descriptor_chain(memory) else {
            break;
        };
        let head = chain.head_index();
        if let Ok(mut reader) = chain.reader(memory) {
            let mut got = 0;
            while got < MAX_CHAIN {
                match reader.read(&mut bytes[got..]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => got += n,
                }
            }
            link.put_from_guest(&bytes[..got]);
        }
        if queue.add_used(memory, head, 0).is_err() {
            break;
        }
        any = true;
    }
    any
}

/// Give the guest what waits for it on a port, as far as it has
/// buffers.
pub fn fill_rx(queue: &mut Queue, memory: &Memory, link: &Link) -> bool {
    let mut any = false;
    while link.has_for_guest() {
        let Some(chain) = queue.pop_descriptor_chain(memory) else {
            break;
        };
        let head = chain.head_index();
        let mut given = 0;
        if let Ok(mut writer) = chain.writer(memory) {
            let bytes = link.take_for_guest(writer.available_bytes().min(MAX_CHAIN));
            if writer.write_all(&bytes).is_ok() {
                given = bytes.len();
            }
        }
        #[allow(clippy::cast_possible_truncation)] // MAX_CHAIN at most
        if queue.add_used(memory, head, given as u32).is_err() {
            break;
        }
        any = true;
    }
    any
}

/// Everything the device has to do, done: what the guest sent taken,
/// what waits for it given, both sides' opens told.
struct Served {
    memory: Arc<Memory>,
    queues: Vec<Option<ActiveQueue>>,
    interrupt: Interrupt,
    control: Arc<Mutex<Control>>,
    ports: Vec<Port>,
}

impl Served {
    fn raise(&self, index: usize) {
        if let Some(Some(q)) = self.queues.get(index) {
            self.interrupt.queue(q.vector);
        }
    }

    fn run(&mut self) {
        let mut control = self.control.lock().unwrap_or_else(PoisonError::into_inner);
        let links: Vec<Arc<Link>> = self.ports.iter().map(|p| p.link.clone()).collect();
        let open = |i: usize| links.get(i).is_some_and(|l| l.connected());
        let mut raised = Vec::new();
        if let Some(Some(q)) = self.queues.get_mut(CONTROL_TX)
            && serve_control_tx(&mut q.queue, &self.memory, &mut control, &open)
        {
            raised.push(CONTROL_TX);
        }
        for (index, link) in links.iter().enumerate() {
            link.set_guest_open(control.guest_open(index));
            control.host(index, link.connected());
        }
        if let Some(Some(q)) = self.queues.get_mut(CONTROL_RX)
            && fill_control_rx(&mut q.queue, &self.memory, &mut control)
        {
            raised.push(CONTROL_RX);
        }
        drop(control);
        for (index, link) in links.iter().enumerate() {
            if let Some(Some(q)) = self.queues.get_mut(tx_queue(index))
                && serve_tx(&mut q.queue, &self.memory, link)
            {
                raised.push(tx_queue(index));
            }
            if let Some(Some(q)) = self.queues.get_mut(rx_queue(index))
                && fill_rx(&mut q.queue, &self.memory, link)
            {
                raised.push(rx_queue(index));
            }
        }
        for index in raised {
            self.raise(index);
        }
    }
}

impl VirtioDevice for Console {
    fn device_type(&self) -> u32 {
        TYPE_CONSOLE
    }

    fn class(&self) -> u32 {
        // A simple communication controller, other.
        0x07_80_00
    }

    fn features(&self) -> u64 {
        FEATURE_MULTIPORT
    }

    fn queue_sizes(&self) -> Vec<u16> {
        vec![QUEUE_SIZE; self.queue_count()]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        // Columns and rows (none), the most ports, no emergency write.
        let max = self
            .control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .max_ports();
        let mut config = [0u8; 12];
        config[4..8].copy_from_slice(&max.to_le_bytes());
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = usize::try_from(offset)
                .ok()
                .and_then(|o| o.checked_add(i))
                .and_then(|at| config.get(at).copied())
                .unwrap_or(0);
        }
    }

    fn write_config(&mut self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, activation: Activation) -> io::Result<()> {
        let Activation {
            memory,
            mut queues,
            interrupt,
            features,
        } = activation;
        if features & FEATURE_MULTIPORT == 0 {
            return Err(io::Error::other("the driver did not take several ports"));
        }
        let mut notifies = Vec::new();
        for (index, q) in queues.iter_mut().enumerate() {
            if let Some(q) = q {
                notifies.push((index, q.notify.try_clone()?));
            }
        }
        let base = queues.len();
        for (i, port) in self.ports.iter().enumerate() {
            notifies.push((base + i, port.link.wake_fd()?));
        }
        let mut served = Served {
            memory,
            queues,
            interrupt,
            control: self.control.clone(),
            ports: self.ports.clone(),
        };
        self.worker = Some(Worker::spawn("virtio-console", notifies, move |_| {
            served.run();
        })?);
        Ok(())
    }

    fn reset(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.stop();
        }
        self.control
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .reset();
        for port in &self.ports {
            port.link.set_guest_open(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::scsi::testing::{Guest, chain};
    use control::{DEVICE_READY, Message, PORT_OPEN, PORT_READY};
    use vm_memory::{Bytes, GuestAddress};

    fn put(guest: &mut Guest, at: u64, bytes: &[u8]) {
        guest.memory.write_slice(bytes, GuestAddress(at)).unwrap();
    }

    fn get(guest: &Guest, at: u64, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        guest.memory.read_slice(&mut out, GuestAddress(at)).unwrap();
        out
    }

    #[test]
    fn the_control_queue_announces_the_ports_by_name() {
        let mut c = Control::new(&["systems.quartz.qsvs.service.0"]);
        let mut tx = Guest::new(16);
        put(
            &mut tx,
            0x8000,
            &Message {
                id: 0,
                event: DEVICE_READY,
                value: 1,
            }
            .bytes(&[]),
        );
        chain(&mut tx, &[(0x8000, 8)], &[]);
        // A short message, then the port's readiness.
        chain(&mut tx, &[(0x8100, 3)], &[]);
        put(
            &mut tx,
            0x8200,
            &Message {
                id: 1,
                event: PORT_READY,
                value: 1,
            }
            .bytes(&[]),
        );
        chain(&mut tx, &[(0x8200, 64)], &[]);
        assert!(serve_control_tx(&mut tx.queue, &tx.memory, &mut c, &|_| {
            true
        }));
        assert_eq!(tx.used().len(), 3);
        assert_eq!(c.passed_over, 1);

        let mut rx = Guest::new(16);
        chain(&mut rx, &[], &[(0x8000, 64)]);
        chain(&mut rx, &[], &[(0x9000, 64)]);
        // A buffer too small for a header is used with nothing.
        chain(&mut rx, &[], &[(0xA000, 4)]);
        assert!(fill_control_rx(&mut rx.queue, &rx.memory, &mut c));
        let used = rx.used();
        assert_eq!(used[0], (0, 8));
        assert_eq!(used[1], (1, 8 + 29));
        assert_eq!(&get(&rx, 0x9000 + 8, 29), b"systems.quartz.qsvs.service.0");
        assert_eq!(used[2], (2, 0));
    }

    #[test]
    fn a_port_carries_bytes_each_way_and_stops_when_the_host_is_slow() {
        let link = Link::new().unwrap();
        let n = link.connect();
        link.set_guest_open(true);
        let mut tx = Guest::new(64);
        for i in 0..40u64 {
            put(
                &mut tx,
                0x1_0000 + i * 0x1000,
                &[u8::try_from(i).unwrap(); 16],
            );
            chain(&mut tx, &[(0x1_0000 + i * 0x1000, 0x1000)], &[]);
        }
        // Sixteen pages fill the link; the rest wait in the queue.
        assert!(serve_tx(&mut tx.queue, &tx.memory, &link));
        assert_eq!(tx.used().len(), 16);
        assert!(!serve_tx(&mut tx.queue, &tx.memory, &link));
        let first = link.from_guest(n, link::CAPACITY).unwrap();
        assert_eq!(first[..16], [0u8; 16]);
        assert!(serve_tx(&mut tx.queue, &tx.memory, &link));
        assert_eq!(tx.used().len(), 32);

        assert!(link.to_guest(n, b"QI frame"));
        let mut rx = Guest::new(16);
        chain(&mut rx, &[], &[(0x8000, 4)]);
        chain(&mut rx, &[], &[(0x9000, 64)]);
        assert!(fill_rx(&mut rx.queue, &rx.memory, &link));
        assert_eq!(rx.used(), [(0, 4), (1, 4)]);
        assert_eq!(get(&rx, 0x8000, 4), b"QI f");
        assert_eq!(get(&rx, 0x9000, 4), b"rame");
        // Nothing waits: no buffer is used.
        assert!(!fill_rx(&mut rx.queue, &rx.memory, &link));

        // With the host's side gone, what the guest sends is dropped.
        link.disconnect(n);
        assert!(serve_tx(&mut tx.queue, &tx.memory, &link));
        assert_eq!(tx.used().len(), 40);
    }

    #[test]
    fn the_configuration_says_the_ports_and_the_guest_s_open_is_kept() {
        let ports = vec![
            Port {
                name: "a".into(),
                link: Arc::new(Link::new().unwrap()),
            },
            Port {
                name: "b".into(),
                link: Arc::new(Link::new().unwrap()),
            },
        ];
        let device = Console::new(ports);
        assert_eq!(device.queue_sizes().len(), 8);
        assert_eq!((rx_queue(0), tx_queue(1)), (4, 7));
        let mut config = [0xFFu8; 12];
        device.read_config(0, &mut config);
        assert_eq!(config[4..8], 3u32.to_le_bytes());
        let mut far = [0xFFu8; 4];
        device.read_config(u64::MAX, &mut far);
        assert_eq!(far, [0; 4]);
        let mut c = device.control.lock().unwrap();
        c.from_guest(
            &Message {
                id: 2,
                event: PORT_OPEN,
                value: 1,
            }
            .bytes(&[]),
            &|_| false,
        );
        assert!(c.guest_open(1));
    }
}
