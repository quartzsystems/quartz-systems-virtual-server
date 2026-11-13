// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! virtio-scsi: one target, a disk at each LUN.
//!
//! - [`cdb`]: the commands, decoded, and the sense data.
//! - [`lun`]: a disk LUN carrying them out.
//! - [`backing`]: the host file or block device behind a LUN.
//!
//! The request queue and the control queue are served on a thread
//! each: a request stuck in the host's I/O does not keep a task
//! management function from being answered, so the guest's error
//! handling runs on instead of hanging. The event queue is never
//! used: nothing is plugged or resized under a running partition.

use std::io::{self, Read, Write};
use std::sync::Arc;

use virtio_queue::{Queue, QueueT};

use crate::Memory;
use crate::virtio::worker::Worker;
use crate::virtio::{Activation, TYPE_SCSI, VirtioDevice};

pub mod backing;
pub mod cdb;
pub mod lun;

use backing::Bounce;
use cdb::Sense;
use lun::{CHUNK_BYTES, Failure, Lun, MAX_TRANSFER_BLOCKS};

/// The most LUNs the target has.
pub const MAX_LUNS: usize = 64;

const CONTROL_QUEUE: usize = 0;
const REQUEST_QUEUE: usize = 2;
const CONTROL_QUEUE_SIZE: u16 = 64;
const EVENT_QUEUE_SIZE: u16 = 64;
const REQUEST_QUEUE_SIZE: u16 = 256;

/// Bytes of a command descriptor block in a request.
const CDB_BYTES: usize = 32;
/// Bytes of sense data in a response.
const SENSE_BYTES: usize = 96;
/// Bytes of a request's header: the LUN, the tag, three bytes of
/// attributes, the CDB.
const REQUEST_HEADER: usize = 8 + 8 + 3 + CDB_BYTES;
/// Bytes of a response: two lengths, the status qualifier, the
/// status, the response code, the sense data.
const RESPONSE_BYTES: usize = 4 + 4 + 2 + 1 + 1 + SENSE_BYTES;

const STATUS_GOOD: u8 = 0;
const STATUS_CHECK_CONDITION: u8 = 2;

const RESPONSE_OK: u8 = 0;
const RESPONSE_OVERRUN: u8 = 1;
const RESPONSE_BAD_TARGET: u8 = 3;
const RESPONSE_FUNCTION_COMPLETE: u8 = 0;
const RESPONSE_FUNCTION_REJECTED: u8 = 11;

const CONTROL_TASK_MANAGEMENT: u32 = 0;
const CONTROL_QUERY: u32 = 1;
const CONTROL_SUBSCRIBE: u32 = 2;

/// The SCSI host.
pub struct Scsi {
    luns: Arc<Vec<Lun>>,
    workers: Vec<Worker>,
}

impl Scsi {
    /// The LUNs, for whoever reports what they were asked for.
    pub fn luns(&self) -> Arc<Vec<Lun>> {
        self.luns.clone()
    }

    /// A host with `luns`, numbered from 0. `None` when there are
    /// none or too many.
    pub fn new(luns: Vec<Lun>) -> Option<Self> {
        (!luns.is_empty() && luns.len() <= MAX_LUNS).then(|| Self {
            luns: Arc::new(luns),
            workers: Vec::new(),
        })
    }

    fn config(&self) -> [u8; 36] {
        let mut c = [0u8; 36];
        let mut put = |at: usize, value: u32| c[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(0, 1); // request queues
        put(4, u32::from(REQUEST_QUEUE_SIZE) - 2); // segments of a request
        put(8, MAX_TRANSFER_BLOCKS); // sectors of a request
        put(12, u32::from(REQUEST_QUEUE_SIZE)); // commands a LUN
        put(16, 0); // no event information
        #[allow(clippy::cast_possible_truncation)] // 96 and 32
        {
            put(20, SENSE_BYTES as u32);
            put(24, CDB_BYTES as u32);
        }
        // Channel 0 and target 0 are the only ones.
        #[allow(clippy::cast_possible_truncation)] // 64 at most
        put(32, self.luns.len() as u32 - 1);
        c
    }
}

/// The LUN a request is for: the first byte is 1, then the target,
/// then the LUN in two bytes.
fn lun_of(address: &[u8]) -> Option<usize> {
    (address[0] == 1 && address[1] == 0)
        .then(|| (usize::from(address[2] & 0x3F) << 8) | usize::from(address[3]))
}

fn response(
    response: u8,
    status: u8,
    sense: Option<Sense>,
    residual: usize,
) -> [u8; RESPONSE_BYTES] {
    let mut r = [0u8; RESPONSE_BYTES];
    let residual = u32::try_from(residual).unwrap_or(u32::MAX);
    r[4..8].copy_from_slice(&residual.to_le_bytes());
    r[10] = status;
    r[11] = response;
    if let Some(sense) = sense {
        let bytes = sense.bytes();
        #[allow(clippy::cast_possible_truncation)] // eighteen
        r[..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        r[12..12 + bytes.len()].copy_from_slice(&bytes);
    }
    r
}

/// Carry out one request. `out` is what the guest sent after the
/// header, `data_in` the guest's room after the response. The response
/// and the bytes written to `data_in`.
pub fn request(
    luns: &[Lun],
    header: &[u8; REQUEST_HEADER],
    out: &mut dyn Read,
    out_bytes: usize,
    data_in: &mut dyn Write,
    in_bytes: usize,
    bounce: &mut Bounce,
) -> ([u8; RESPONSE_BYTES], usize) {
    let Some(lun) = lun_of(&header[..8]).and_then(|n| luns.get(n)) else {
        return (response(RESPONSE_BAD_TARGET, STATUS_GOOD, None, 0), 0);
    };
    let outcome = match cdb::decode(&header[19..]) {
        Ok(command) => lun.execute(command, luns.len(), out, data_in, bounce),
        Err(sense) => {
            lun.refused();
            Err(Failure::Check(sense))
        }
    };
    match outcome {
        Ok(given) => {
            // What of the guest's buffers was not used, whichever way
            // the data went.
            let residual = if given > 0 {
                in_bytes - given.min(in_bytes)
            } else {
                0
            };
            let _ = out_bytes;
            (response(RESPONSE_OK, STATUS_GOOD, None, residual), given)
        }
        Err(Failure::Check(sense)) => (
            response(RESPONSE_OK, STATUS_CHECK_CONDITION, Some(sense), in_bytes),
            0,
        ),
        Err(Failure::Overrun) => (response(RESPONSE_OVERRUN, STATUS_GOOD, None, 0), 0),
    }
}

/// Serve every request the guest has put in the request queue.
/// Whether any was.
pub fn serve_requests(
    queue: &mut Queue,
    memory: &Memory,
    luns: &[Lun],
    bounce: &mut Bounce,
) -> bool {
    let mut any = false;
    while let Some(chain) = queue.pop_descriptor_chain(memory) {
        let head = chain.head_index();
        let mut used = 0;
        // A chain that is not a request is used with nothing written.
        if let (Ok(mut reader), Ok(mut writer)) =
            (chain.clone().reader(memory), chain.writer(memory))
        {
            let mut header = [0u8; REQUEST_HEADER];
            if reader.read_exact(&mut header).is_ok()
                && writer.available_bytes() >= RESPONSE_BYTES
                && let Ok(mut data_in) = writer.split_at(RESPONSE_BYTES)
            {
                let (out_bytes, in_bytes) = (reader.available_bytes(), data_in.available_bytes());
                let (bytes, given) = request(
                    luns,
                    &header,
                    &mut reader,
                    out_bytes,
                    &mut data_in,
                    in_bytes,
                    bounce,
                );
                if writer.write_all(&bytes).is_ok() {
                    used = RESPONSE_BYTES + given;
                }
            }
        }
        let used = u32::try_from(used).unwrap_or(u32::MAX);
        if queue.add_used(memory, head, used).is_err() {
            break;
        }
        any = true;
    }
    any
}

/// Answer every request the guest has put in the control queue.
/// Whether any was.
pub fn serve_control(queue: &mut Queue, memory: &Memory) -> bool {
    let mut any = false;
    while let Some(chain) = queue.pop_descriptor_chain(memory) {
        let head = chain.head_index();
        let mut used = 0;
        if let (Ok(mut reader), Ok(mut writer)) =
            (chain.clone().reader(memory), chain.writer(memory))
        {
            let mut kind = [0u8; 4];
            let answer: &[u8] = match reader
                .read_exact(&mut kind)
                .map(|()| u32::from_le_bytes(kind))
            {
                // Every request ends before the next is taken, so a
                // task named here is no longer the device's: aborting
                // or resetting has nothing left to do.
                Ok(CONTROL_TASK_MANAGEMENT) => &[RESPONSE_FUNCTION_COMPLETE],
                // No asynchronous event is ever reported.
                Ok(CONTROL_QUERY | CONTROL_SUBSCRIBE) => &[0, 0, 0, 0, RESPONSE_OK],
                _ => &[RESPONSE_FUNCTION_REJECTED],
            };
            if writer.available_bytes() >= answer.len() && writer.write_all(answer).is_ok() {
                used = answer.len();
            }
        }
        let used = u32::try_from(used).unwrap_or(u32::MAX);
        if queue.add_used(memory, head, used).is_err() {
            break;
        }
        any = true;
    }
    any
}

impl VirtioDevice for Scsi {
    fn device_type(&self) -> u32 {
        TYPE_SCSI
    }

    fn class(&self) -> u32 {
        0x01_00_00
    }

    fn features(&self) -> u64 {
        0
    }

    fn queue_sizes(&self) -> Vec<u16> {
        vec![CONTROL_QUEUE_SIZE, EVENT_QUEUE_SIZE, REQUEST_QUEUE_SIZE]
    }

    fn read_config(&self, offset: u64, data: &mut [u8]) {
        let config = self.config();
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = usize::try_from(offset)
                .ok()
                .and_then(|o| o.checked_add(i))
                .and_then(|at| config.get(at))
                .copied()
                .unwrap_or(0);
        }
    }

    /// The driver may write the sense and CDB sizes; they are fixed
    /// here at what every driver writes, so a write changes nothing.
    fn write_config(&mut self, _offset: u64, _data: &[u8]) {}

    fn activate(&mut self, activation: Activation) -> io::Result<()> {
        let Activation {
            memory,
            mut queues,
            interrupt,
            ..
        } = activation;
        let mut take = |index: usize| queues.get_mut(index).and_then(Option::take);
        let Some(requests) = take(REQUEST_QUEUE) else {
            return Err(io::Error::other("the request queue is not enabled"));
        };
        let control = take(CONTROL_QUEUE);

        let luns = self.luns.clone();
        let (request_memory, request_interrupt) = (memory.clone(), interrupt.clone());
        let mut queue = requests.queue;
        let vector = requests.vector;
        let mut bounce = Bounce::new(CHUNK_BYTES);
        self.workers.push(Worker::spawn(
            "virtio-scsi",
            vec![(REQUEST_QUEUE, requests.notify)],
            move |_| {
                if serve_requests(&mut queue, &request_memory, &luns, &mut bounce) {
                    request_interrupt.queue(vector);
                }
            },
        )?);
        if let Some(control) = control {
            let mut queue = control.queue;
            let vector = control.vector;
            self.workers.push(Worker::spawn(
                "virtio-scsi-ctl",
                vec![(CONTROL_QUEUE, control.notify)],
                move |_| {
                    if serve_control(&mut queue, &memory) {
                        interrupt.queue(vector);
                    }
                },
            )?);
        }
        Ok(())
    }

    fn reset(&mut self) {
        for mut worker in self.workers.drain(..) {
            worker.stop();
        }
    }
}

/// A guest's side of a queue, for the tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use vm_memory::{Bytes, GuestAddress};

    const DESC: u32 = 0x1000;
    const AVAIL: u32 = 0x2000;
    const USED: u32 = 0x3000;

    /// Guest memory with one queue in it.
    pub struct Guest {
        pub memory: Memory,
        pub queue: Queue,
        size: u16,
        next_desc: u16,
        avail: u16,
    }

    impl Guest {
        /// A megabyte of memory and a queue of `size`.
        pub fn new(size: u16) -> Self {
            let memory = Memory::from_ranges(&[(GuestAddress(0), 0x10_0000)]).unwrap();
            let mut queue = Queue::new(size).unwrap();
            queue.set_desc_table_address(Some(DESC), Some(0));
            queue.set_avail_ring_address(Some(AVAIL), Some(0));
            queue.set_used_ring_address(Some(USED), Some(0));
            queue.set_ready(true);
            assert!(queue.is_valid(&memory));
            Self {
                memory,
                queue,
                size,
                next_desc: 0,
                avail: 0,
            }
        }

        /// Write one descriptor as given, at the next free index.
        pub fn descriptor(&mut self, addr: u64, len: u32, flags: u16, next: u16) -> u16 {
            let index = self.next_desc;
            let at = u64::from(DESC) + u64::from(index) * 16;
            self.memory.write_obj(addr, GuestAddress(at)).unwrap();
            self.memory.write_obj(len, GuestAddress(at + 8)).unwrap();
            self.memory.write_obj(flags, GuestAddress(at + 12)).unwrap();
            self.memory.write_obj(next, GuestAddress(at + 14)).unwrap();
            self.next_desc += 1;
            index
        }

        /// Make the chain starting at `head` available.
        pub fn offer(&mut self, head: u16) {
            let slot = u64::from(AVAIL) + 4 + u64::from(self.avail % self.size) * 2;
            self.memory.write_obj(head, GuestAddress(slot)).unwrap();
            self.avail = self.avail.wrapping_add(1);
            self.memory
                .write_obj(self.avail, GuestAddress(u64::from(AVAIL) + 2))
                .unwrap();
        }

        /// The used ring: each chain's head and the bytes written.
        pub fn used(&self) -> Vec<(u32, u32)> {
            let count: u16 = self
                .memory
                .read_obj(GuestAddress(u64::from(USED) + 2))
                .unwrap();
            (0..u64::from(count))
                .map(|i| {
                    let at = u64::from(USED) + 4 + (i % u64::from(self.size)) * 8;
                    (
                        self.memory.read_obj(GuestAddress(at)).unwrap(),
                        self.memory.read_obj(GuestAddress(at + 4)).unwrap(),
                    )
                })
                .collect()
        }
    }

    /// Offer a chain of readable buffers, then writable ones. Its head.
    pub fn chain(guest: &mut Guest, readable: &[(u64, u32)], writable: &[(u64, u32)]) -> u16 {
        let total = readable.len() + writable.len();
        let head = guest.next_desc;
        for (i, (addr, len)) in readable.iter().chain(writable).enumerate() {
            let mut flags = 0;
            if i >= readable.len() {
                flags |= 2;
            }
            if i + 1 < total {
                flags |= 1;
            }
            let next = guest.next_desc + 1;
            guest.descriptor(*addr, *len, flags, next);
        }
        guest.offer(head);
        head
    }
}

#[cfg(test)]
mod tests {
    use super::backing::memory::MemoryStore;
    use super::testing::{Guest, chain};
    use super::*;
    use vm_memory::{Bytes, GuestAddress};

    const HEADER_AT: u64 = 0x8000;
    const OUT_AT: u64 = 0x9000;
    const RESPONSE_AT: u64 = 0xA000;
    const IN_AT: u64 = 0xB000;

    fn luns() -> Vec<Lun> {
        vec![
            Lun::new(Box::new(MemoryStore::new(64 * 512, true)), "LS0").unwrap(),
            Lun::new(Box::new(MemoryStore::new(16 * 512, false)), "D1").unwrap(),
        ]
    }

    fn header(guest: &Guest, lun: u16, cdb: &[u8]) {
        let mut h = [0u8; REQUEST_HEADER];
        h[0] = 1;
        h[2] = 0x40 | lun.to_be_bytes()[0];
        h[3] = lun.to_be_bytes()[1];
        h[19..19 + cdb.len()].copy_from_slice(cdb);
        guest
            .memory
            .write_slice(&h, GuestAddress(HEADER_AT))
            .unwrap();
    }

    fn answer(guest: &Guest) -> (u8, u8, [u8; 18]) {
        let mut r = [0u8; RESPONSE_BYTES];
        guest
            .memory
            .read_slice(&mut r, GuestAddress(RESPONSE_AT))
            .unwrap();
        (r[11], r[10], r[12..30].try_into().unwrap())
    }

    fn rw10(opcode: u8, lba: u32, blocks: u16) -> [u8; 10] {
        let mut c = [0u8; 10];
        c[0] = opcode;
        c[2..6].copy_from_slice(&lba.to_be_bytes());
        c[7..9].copy_from_slice(&blocks.to_be_bytes());
        c
    }

    #[test]
    fn a_write_and_a_read_go_through_the_queue() {
        let luns = luns();
        let mut guest = Guest::new(16);
        let mut bounce = Bounce::new(4096);
        let data: Vec<u8> = (0..1024u32).map(|i| (i % 199) as u8).collect();
        guest
            .memory
            .write_slice(&data, GuestAddress(OUT_AT))
            .unwrap();

        header(&guest, 1, &rw10(0x2A, 3, 2));
        chain(
            &mut guest,
            &[(HEADER_AT, 51), (OUT_AT, 1024)],
            &[(RESPONSE_AT, 108)],
        );
        assert!(serve_requests(
            &mut guest.queue,
            &guest.memory,
            &luns,
            &mut bounce
        ));
        assert_eq!(guest.used(), [(0, 108)]);
        assert_eq!(answer(&guest).0, RESPONSE_OK);
        assert_eq!(answer(&guest).1, STATUS_GOOD);

        header(&guest, 1, &rw10(0x28, 3, 2));
        // The data comes back in two buffers of the guest's.
        chain(
            &mut guest,
            &[(HEADER_AT, 51)],
            &[(RESPONSE_AT, 108), (IN_AT, 512), (IN_AT + 0x1000, 512)],
        );
        assert!(serve_requests(
            &mut guest.queue,
            &guest.memory,
            &luns,
            &mut bounce
        ));
        assert_eq!(guest.used()[1], (3, 108 + 1024));
        let mut back = vec![0u8; 512];
        guest
            .memory
            .read_slice(&mut back, GuestAddress(IN_AT + 0x1000))
            .unwrap();
        assert_eq!(back, data[512..]);
        // The queue is empty again.
        assert!(!serve_requests(
            &mut guest.queue,
            &guest.memory,
            &luns,
            &mut bounce
        ));
    }

    #[test]
    fn out_of_range_and_unknown_requests_are_refused_in_the_response() {
        let luns = luns();
        let mut guest = Guest::new(16);
        let mut bounce = Bounce::new(4096);
        let mut ask = |guest: &mut Guest, lun: u16, cdb: &[u8], room: u32| {
            header(guest, lun, cdb);
            chain(
                guest,
                &[(HEADER_AT, 51)],
                &[(RESPONSE_AT, 108), (IN_AT, room)],
            );
            assert!(serve_requests(
                &mut guest.queue,
                &guest.memory,
                &luns,
                &mut bounce
            ));
            answer(guest)
        };
        // Past the end of LUN 1, which has sixteen blocks.
        let (response, status, sense) = ask(&mut guest, 1, &rw10(0x28, 16, 1), 512);
        assert_eq!((response, status), (RESPONSE_OK, STATUS_CHECK_CONDITION));
        assert_eq!((sense[2], sense[12]), (5, 0x21));
        // A command no disk takes.
        let (_, status, sense) = ask(&mut guest, 0, &[0xEE; 16], 512);
        assert_eq!((status, sense[12]), (STATUS_CHECK_CONDITION, 0x20));
        // A LUN that is not there, and a target that is not.
        assert_eq!(
            ask(&mut guest, 2, &rw10(0x28, 0, 1), 512).0,
            RESPONSE_BAD_TARGET
        );
        assert_eq!(
            ask(&mut guest, 0x3FFF, &[0x00; 6], 0).0,
            RESPONSE_BAD_TARGET
        );
        // A read into too little room.
        assert_eq!(
            ask(&mut guest, 0, &rw10(0x28, 0, 2), 512).0,
            RESPONSE_OVERRUN
        );
        let used: Vec<u32> = guest.used().iter().map(|u| u.1).collect();
        assert_eq!(used, [108; 5]);
    }

    #[test]
    fn chains_that_are_no_request_are_used_and_harm_nothing() {
        let luns = luns();
        let mut guest = Guest::new(16);
        let mut bounce = Bounce::new(4096);
        // Too short a header; no room for a response; nothing writable;
        // a buffer outside memory; a chain that loops.
        chain(&mut guest, &[(HEADER_AT, 20)], &[(RESPONSE_AT, 108)]);
        chain(&mut guest, &[(HEADER_AT, 51)], &[(RESPONSE_AT, 50)]);
        chain(&mut guest, &[(HEADER_AT, 51)], &[]);
        chain(&mut guest, &[(0xFFFF_FFFF_0000, 51)], &[(RESPONSE_AT, 108)]);
        let looped = guest.descriptor(HEADER_AT, 51, 1, 8);
        assert_eq!(looped, 7);
        guest.descriptor(RESPONSE_AT, 108, 1 | 2, 7);
        guest.offer(looped);
        serve_requests(&mut guest.queue, &guest.memory, &luns, &mut bounce);
        let written: Vec<u32> = guest.used().iter().map(|u| u.1).collect();
        // The loop is followed as far as the queue is long and no
        // further; what it holds is a request for no target.
        assert_eq!(written, [0, 0, 0, 0, 108]);
        let refused: u8 = guest
            .memory
            .read_obj(GuestAddress(RESPONSE_AT + 11))
            .unwrap();
        assert_eq!(refused, RESPONSE_BAD_TARGET);
        // A good request after them is still served.
        header(&guest, 0, &[0x00; 6]);
        chain(&mut guest, &[(HEADER_AT, 51)], &[(RESPONSE_AT, 108)]);
        serve_requests(&mut guest.queue, &guest.memory, &luns, &mut bounce);
        assert_eq!(guest.used().last().map(|u| u.1), Some(108));
    }

    #[test]
    fn task_management_is_always_answered() {
        let mut guest = Guest::new(16);
        let put = |guest: &Guest, kind: u32| {
            guest
                .memory
                .write_obj(kind, GuestAddress(HEADER_AT))
                .unwrap();
        };
        put(&guest, CONTROL_TASK_MANAGEMENT);
        chain(&mut guest, &[(HEADER_AT, 24)], &[(RESPONSE_AT, 1)]);
        assert!(serve_control(&mut guest.queue, &guest.memory));
        let done: u8 = guest.memory.read_obj(GuestAddress(RESPONSE_AT)).unwrap();
        assert_eq!(done, RESPONSE_FUNCTION_COMPLETE);
        put(&guest, CONTROL_QUERY);
        chain(&mut guest, &[(HEADER_AT, 16)], &[(RESPONSE_AT, 5)]);
        put(&guest, 77);
        chain(&mut guest, &[(HEADER_AT, 16)], &[(RESPONSE_AT, 5)]);
        chain(&mut guest, &[(HEADER_AT, 2)], &[(RESPONSE_AT, 5)]);
        chain(&mut guest, &[(HEADER_AT, 16)], &[]);
        assert!(serve_control(&mut guest.queue, &guest.memory));
        let written: Vec<u32> = guest.used().iter().map(|u| u.1).collect();
        // The query was read as 77 by then: both are rejected in one byte.
        assert_eq!(written, [1, 1, 1, 1, 0]);
    }

    #[test]
    fn the_configuration_names_one_target_and_its_luns() {
        let scsi = Scsi::new(luns()).unwrap();
        let mut c = [0u8; 40];
        scsi.read_config(0, &mut c);
        assert_eq!(u32::from_le_bytes(c[0..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(c[20..24].try_into().unwrap()), 96);
        assert_eq!(u32::from_le_bytes(c[24..28].try_into().unwrap()), 32);
        assert_eq!(u16::from_le_bytes(c[30..32].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(c[32..36].try_into().unwrap()), 1);
        assert_eq!(c[36..], [0; 4]);
        assert!(Scsi::new(Vec::new()).is_none());
    }
}
