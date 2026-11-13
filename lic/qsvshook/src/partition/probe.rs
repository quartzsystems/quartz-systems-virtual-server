// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The requests no guest kernel sends. A Linux guest never asks for a
//! block beyond its disk or puts a request in the queue that is not
//! one, so the suite cannot show from a guest that the device refuses
//! them. This makes them itself: a virtio queue in memory of its own,
//! the virtio-scsi device's own request handling, and behind it the
//! host device a partition would have, opened as `qsvmm` opens it.

use std::path::Path;

use devices::Memory;
use devices::virtio::scsi::backing::{Bounce, HostStore};
use devices::virtio::scsi::lun::Lun;
use devices::virtio::scsi::serve_requests;
use virtio_queue::{Queue, QueueT};
use vm_memory::{Bytes, GuestAddress};

const DESC: u32 = 0x1000;
const AVAIL: u32 = 0x2000;
const USED: u32 = 0x3000;
const HEADER_AT: u64 = 0x8000;
const OUT_AT: u64 = 0x9000;
const RESPONSE_AT: u64 = 0xA000;
const IN_AT: u64 = 0xB000;
const QUEUE_SIZE: u16 = 64;
const HEADER_BYTES: u32 = 51;
const RESPONSE_BYTES: u32 = 108;
const BLOCK: u32 = 512;

const STATUS_GOOD: u8 = 0;
const STATUS_CHECK: u8 = 2;
const RESPONSE_OK: u8 = 0;
const RESPONSE_BAD_TARGET: u8 = 3;
const SENSE_ILLEGAL_REQUEST: u8 = 5;
const LBA_OUT_OF_RANGE: u8 = 0x21;
const INVALID_OPCODE: u8 = 0x20;

/// What a request came back with.
struct Answer {
    used: u32,
    response: u8,
    status: u8,
    sense: (u8, u8),
    data: Vec<u8>,
}

struct Probe {
    memory: Memory,
    queue: Queue,
    luns: Vec<Lun>,
    bounce: Bounce,
    descriptors: u16,
    offered: u16,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Probe {
    fn new(lun: Lun) -> Result<Self, String> {
        let memory = Memory::from_ranges(&[(GuestAddress(0), 0x10_0000)]).map_err(text)?;
        let mut queue = Queue::new(QUEUE_SIZE).map_err(text)?;
        queue.set_desc_table_address(Some(DESC), Some(0));
        queue.set_avail_ring_address(Some(AVAIL), Some(0));
        queue.set_used_ring_address(Some(USED), Some(0));
        queue.set_ready(true);
        Ok(Self {
            memory,
            queue,
            luns: vec![lun],
            bounce: Bounce::new(1 << 16),
            descriptors: 0,
            offered: 0,
        })
    }

    /// Offer a chain of readable buffers, then writable ones, and have
    /// the device serve it. The bytes the device says it wrote.
    fn offer(&mut self, readable: &[(u64, u32)], writable: &[(u64, u32)]) -> Result<u32, String> {
        let total = readable.len() + writable.len();
        // The descriptor table is written over from its start once it
        // is full; each chain is served before the next is made.
        if usize::from(self.descriptors) + total > usize::from(QUEUE_SIZE) {
            self.descriptors = 0;
        }
        let head = self.descriptors;
        for (i, (addr, len)) in readable.iter().chain(writable).enumerate() {
            let at = u64::from(DESC) + u64::from(self.descriptors) * 16;
            let flags = u16::from(i + 1 < total) | (u16::from(i >= readable.len()) << 1);
            let m = &self.memory;
            m.write_obj(*addr, GuestAddress(at)).map_err(text)?;
            m.write_obj(*len, GuestAddress(at + 8)).map_err(text)?;
            m.write_obj(flags, GuestAddress(at + 12)).map_err(text)?;
            m.write_obj(self.descriptors + 1, GuestAddress(at + 14))
                .map_err(text)?;
            self.descriptors += 1;
        }
        let slot = u64::from(AVAIL) + 4 + u64::from(self.offered % QUEUE_SIZE) * 2;
        self.memory
            .write_obj(head, GuestAddress(slot))
            .map_err(text)?;
        self.offered = self.offered.wrapping_add(1);
        self.memory
            .write_obj(self.offered, GuestAddress(u64::from(AVAIL) + 2))
            .map_err(text)?;
        serve_requests(&mut self.queue, &self.memory, &self.luns, &mut self.bounce);
        let entry = u64::from(USED) + 4 + u64::from(self.offered.wrapping_sub(1) % QUEUE_SIZE) * 8;
        self.memory.read_obj(GuestAddress(entry + 4)).map_err(text)
    }

    /// A request for `lun`: the command, its data-out, and room for
    /// `room` bytes of data-in.
    fn ask(&mut self, lun: u8, cdb: &[u8], out: &[u8], room: u32) -> Result<Answer, String> {
        let mut header = [0u8; HEADER_BYTES as usize];
        header[0] = 1;
        header[2] = 0x40;
        header[3] = lun;
        header[19..19 + cdb.len()].copy_from_slice(cdb);
        let m = &self.memory;
        m.write_slice(&header, GuestAddress(HEADER_AT))
            .map_err(text)?;
        m.write_slice(out, GuestAddress(OUT_AT)).map_err(text)?;
        m.write_slice(&[0xEE; RESPONSE_BYTES as usize], GuestAddress(RESPONSE_AT))
            .map_err(text)?;
        let mut readable = vec![(HEADER_AT, HEADER_BYTES)];
        if !out.is_empty() {
            readable.push((OUT_AT, u32::try_from(out.len()).map_err(text)?));
        }
        let mut writable = vec![(RESPONSE_AT, RESPONSE_BYTES)];
        if room > 0 {
            writable.push((IN_AT, room));
        }
        let used = self.offer(&readable, &writable)?;
        let mut response = [0u8; RESPONSE_BYTES as usize];
        self.memory
            .read_slice(&mut response, GuestAddress(RESPONSE_AT))
            .map_err(text)?;
        let mut data = vec![0u8; used.saturating_sub(RESPONSE_BYTES) as usize];
        self.memory
            .read_slice(&mut data, GuestAddress(IN_AT))
            .map_err(text)?;
        Ok(Answer {
            used,
            response: response[11],
            status: response[10],
            sense: (response[12 + 2], response[12 + 12]),
            data,
        })
    }
}

fn transfer16(opcode: u8, lba: u64, blocks: u32) -> [u8; 16] {
    let mut cdb = [0u8; 16];
    cdb[0] = opcode;
    cdb[2..10].copy_from_slice(&lba.to_be_bytes());
    cdb[10..14].copy_from_slice(&blocks.to_be_bytes());
    cdb
}

fn refused(what: &str, answer: &Answer, asc: u8) -> Result<(), String> {
    let wanted = (RESPONSE_OK, STATUS_CHECK, (SENSE_ILLEGAL_REQUEST, asc));
    let got = (answer.response, answer.status, answer.sense);
    if got == wanted && answer.used == RESPONSE_BYTES {
        Ok(())
    } else {
        Err(format!(
            "{what}: response {}, status {}, sense {:#04x}/{:#04x}, {} bytes used; \
             a check condition with sense 0x05/{asc:#04x} was expected",
            got.0, got.1, got.2.0, got.2.1, answer.used
        ))
    }
}

/// Make the requests of `device`. What was seen, a line each.
pub fn probe(device: &Path) -> Result<Vec<String>, String> {
    let store = HostStore::open(device).map_err(|e| format!("{}: {e}", device.display()))?;
    let lun = Lun::new(Box::new(store), "PROBE").ok_or("not a disk unit")?;
    let blocks = lun.blocks();
    let last = blocks - 1;
    let mut p = Probe::new(lun)?;
    let mut lines = Vec::new();

    // The last block reads, and is what the rest must leave alone.
    let before = p.ask(0, &transfer16(0x88, last, 1), &[], BLOCK)?;
    if (before.response, before.status) != (RESPONSE_OK, STATUS_GOOD) || before.data.len() != 512 {
        return Err(format!(
            "reading the last block: response {}, status {}, {} bytes",
            before.response,
            before.status,
            before.data.len()
        ));
    }
    lines.push(format!("the unit has {blocks} blocks; the last one reads"));

    beyond_the_end(&mut p, blocks, &mut lines)?;
    no_requests(&mut p, &mut lines)?;

    let after = p.ask(0, &transfer16(0x88, last, 1), &[], BLOCK)?;
    if after.data != before.data {
        return Err("the last block changed".into());
    }
    let counts = p.luns[0].counts();
    if counts.writes != 0 {
        return Err(format!("{} writes reached the unit", counts.writes));
    }
    lines.push(format!(
        "the last block is unchanged; {} requests refused, {} reads, no write reached the unit",
        counts.refused, counts.reads
    ));
    Ok(lines)
}

/// Requests for blocks the unit does not have, a command it does not
/// take, a LUN that is not there.
fn beyond_the_end(p: &mut Probe, blocks: u64, lines: &mut Vec<String>) -> Result<(), String> {
    let last = blocks - 1;
    let pattern = [0xA5u8; 1024];
    let beyond = [
        (
            "a read one block past the end",
            transfer16(0x88, blocks, 1),
            &pattern[..0],
            BLOCK,
        ),
        (
            "a write one block past the end",
            transfer16(0x8A, blocks, 1),
            &pattern[..512],
            0,
        ),
        (
            "a write across the end",
            transfer16(0x8A, last, 2),
            &pattern[..],
            0,
        ),
        (
            "a read at the largest address",
            transfer16(0x88, u64::MAX, 1),
            &pattern[..0],
            BLOCK,
        ),
        (
            "a verify across the end",
            transfer16(0x8F, last, 2),
            &pattern[..0],
            0,
        ),
    ];
    for (what, cdb, out, room) in beyond {
        refused(what, &p.ask(0, &cdb, out, room)?, LBA_OUT_OF_RANGE)?;
        lines.push(format!(
            "{what} is refused: logical block address out of range"
        ));
    }
    refused(
        "an unknown command",
        &p.ask(0, &[0xEE; 16], &[], BLOCK)?,
        INVALID_OPCODE,
    )?;
    lines.push("an unknown command is refused: invalid command operation code".into());

    let nowhere = p.ask(5, &transfer16(0x88, 0, 1), &[], BLOCK)?;
    if nowhere.response != RESPONSE_BAD_TARGET {
        return Err(format!(
            "a LUN that is not there: response {}",
            nowhere.response
        ));
    }
    lines.push("a LUN that is not there is a bad target".into());
    Ok(())
}

/// A chain: what it is, its readable buffers, its writable ones.
type Chain = (&'static str, Vec<(u64, u32)>, Vec<(u64, u32)>);

/// Chains that are no request: nothing is written for any of them.
fn no_requests(p: &mut Probe, lines: &mut Vec<String>) -> Result<(), String> {
    let chains: [Chain; 4] = [
        (
            "a header cut short",
            vec![(HEADER_AT, 20)],
            vec![(RESPONSE_AT, RESPONSE_BYTES)],
        ),
        (
            "no room for a response",
            vec![(HEADER_AT, HEADER_BYTES)],
            vec![(RESPONSE_AT, 50)],
        ),
        (
            "nothing to write to",
            vec![(HEADER_AT, HEADER_BYTES)],
            vec![],
        ),
        (
            "a buffer outside memory",
            vec![(0xFFFF_FFFF_0000, HEADER_BYTES)],
            vec![(RESPONSE_AT, RESPONSE_BYTES)],
        ),
    ];
    for (what, readable, writable) in chains {
        let used = p.offer(&readable, &writable)?;
        if used != 0 {
            return Err(format!("{what}: the device wrote {used} bytes"));
        }
        lines.push(format!("{what} is used with nothing written"));
    }
    Ok(())
}
