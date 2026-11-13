// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! virtio-rng: the guest's buffers filled from the host's random
//! source, a page at most a buffer.

use std::fs::File;
use std::io::{self, Read, Write};
use std::sync::Arc;

use virtio_queue::{Queue, QueueT};

use crate::Memory;
use crate::virtio::worker::Worker;
use crate::virtio::{Activation, Interrupt, TYPE_RNG, VirtioDevice};

/// The queue's size.
const QUEUE_SIZE: u16 = 64;
/// The most bytes given for one buffer.
const MAX_BYTES: usize = 4096;

/// The entropy device.
pub struct Rng {
    source: Arc<File>,
    worker: Option<Worker>,
}

impl Rng {
    /// A device that reads `source`, the host's random device, opened
    /// by the caller.
    pub fn new(source: File) -> Self {
        Self {
            source: Arc::new(source),
            worker: None,
        }
    }
}

/// Fill every buffer the guest has put in the queue. Whether any was.
pub fn fill(queue: &mut Queue, memory: &Memory, mut source: &File) -> bool {
    let mut any = false;
    let mut bytes = [0u8; MAX_BYTES];
    while let Some(chain) = queue.pop_descriptor_chain(memory) {
        let head = chain.head_index();
        let mut given = 0;
        if let Ok(mut writer) = chain.writer(memory) {
            let want = writer.available_bytes().min(MAX_BYTES);
            if source.read_exact(&mut bytes[..want]).is_ok()
                && writer.write_all(&bytes[..want]).is_ok()
            {
                given = want;
            }
        }
        #[allow(clippy::cast_possible_truncation)] // a page at most
        if queue.add_used(memory, head, given as u32).is_err() {
            break;
        }
        any = true;
    }
    any
}

impl VirtioDevice for Rng {
    fn device_type(&self) -> u32 {
        TYPE_RNG
    }

    fn class(&self) -> u32 {
        0x00_FF_00
    }

    fn features(&self) -> u64 {
        0
    }

    fn queue_sizes(&self) -> Vec<u16> {
        vec![QUEUE_SIZE]
    }

    fn read_config(&self, _offset: u64, data: &mut [u8]) {
        data.fill(0);
    }

    fn write_config(&mut self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, activation: Activation) -> io::Result<()> {
        let Activation {
            memory,
            mut queues,
            interrupt,
            ..
        } = activation;
        let Some(active) = queues.first_mut().and_then(Option::take) else {
            return Err(io::Error::other("the queue is not enabled"));
        };
        let source = self.source.clone();
        let mut queue = active.queue;
        let vector = active.vector;
        let interrupt: Interrupt = interrupt;
        self.worker = Some(Worker::spawn(
            "virtio-rng",
            vec![(0, active.notify)],
            move |_| {
                if fill(&mut queue, &memory, &source) {
                    interrupt.queue(vector);
                }
            },
        )?);
        Ok(())
    }

    fn reset(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::scsi::testing::{Guest, chain};
    use vm_memory::{Bytes, GuestAddress};

    #[test]
    fn a_buffer_is_filled_and_a_large_one_only_by_a_page() {
        let mut guest = Guest::new(16);
        let source = File::open("/dev/urandom").unwrap();
        chain(&mut guest, &[], &[(0x8000, 64)]);
        chain(&mut guest, &[], &[(0x1_0000, 0x4000)]);
        assert!(fill(&mut guest.queue, &guest.memory, &source));
        assert_eq!(guest.used(), [(0, 64), (1, 4096)]);
        let mut got = [0u8; 64];
        guest
            .memory
            .read_slice(&mut got, GuestAddress(0x8000))
            .unwrap();
        assert_ne!(got, [0u8; 64]);
        // Beyond the page nothing was written.
        let mut rest = [0u8; 16];
        guest
            .memory
            .read_slice(&mut rest, GuestAddress(0x1_1000))
            .unwrap();
        assert_eq!(rest, [0u8; 16]);
        // A chain with nothing to write to is used with no bytes.
        chain(&mut guest, &[(0x9000, 8)], &[]);
        assert!(fill(&mut guest.queue, &guest.memory, &source));
        assert_eq!(guest.used().last(), Some(&(2, 0)));
    }
}
