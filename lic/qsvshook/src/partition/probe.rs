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
use devices::virtio::scsi::Unit;
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
    marks: u8,
    data: Vec<u8>,
}

struct Probe {
    memory: Memory,
    queue: Queue,
    luns: Vec<std::sync::Arc<dyn Unit>>,
    bounce: Bounce,
    descriptors: u16,
    offered: u16,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Probe {
    fn new(lun: Lun) -> Result<Self, String> {
        Self::of(std::sync::Arc::new(lun))
    }

    fn of(unit: std::sync::Arc<dyn Unit>) -> Result<Self, String> {
        let memory = Memory::from_ranges(&[(GuestAddress(0), 0x10_0000)]).map_err(text)?;
        let mut queue = Queue::new(QUEUE_SIZE).map_err(text)?;
        queue.set_desc_table_address(Some(DESC), Some(0));
        queue.set_avail_ring_address(Some(AVAIL), Some(0));
        queue.set_used_ring_address(Some(USED), Some(0));
        queue.set_ready(true);
        Ok(Self {
            memory,
            queue,
            luns: vec![unit],
            bounce: Bounce::new(1 << 20),
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
        let given = used.saturating_sub(RESPONSE_BYTES).min(room);
        let mut data = vec![0u8; given as usize];
        self.memory
            .read_slice(&mut data, GuestAddress(IN_AT))
            .map_err(text)?;
        Ok(Answer {
            used,
            response: response[11],
            status: response[10],
            sense: (response[12 + 2] & 0x0F, response[12 + 12]),
            marks: response[12 + 2] & 0xE0,
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

/// A whole image read through an optical drive and compared with the
/// file, an eject, another image loaded, a write and malformed commands
/// refused. `first` and `second` are images of the media library.
pub fn optical(first: &Path, second: &Path) -> Result<Vec<String>, String> {
    use devices::virtio::scsi::backing::ImageFile;
    use devices::virtio::scsi::optical::Optical;
    let open = |p: &Path| {
        std::fs::File::open(p)
            .and_then(ImageFile::new)
            .map_err(|e| format!("{}: {e}", p.display()))
    };
    let drive = std::sync::Arc::new(Optical::new("PROBE-OPT").ok_or("a drive")?);
    drive.load(Box::new(open(first)?), "FIRST")?;
    let mut p = Probe::of(drive.clone())?;
    let mut lines = Vec::new();
    // The power-on and the load are told, once each.
    let mut told = 0;
    while p.ask(0, &[0, 0, 0, 0, 0, 0], &[], 0)?.status == STATUS_CHECK {
        told += 1;
        if told > 3 {
            return Err("the drive never became ready".into());
        }
    }
    let inquiry = p.ask(0, &[0x12, 0, 0, 0, 36, 0], &[], 36)?;
    if inquiry.data.first() != Some(&5) {
        return Err(format!("the inquiry said {:?}", inquiry.data));
    }
    let cap = p.ask(0, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[], 8)?;
    let blocks = u64::from(u32::from_be_bytes([
        cap.data[0],
        cap.data[1],
        cap.data[2],
        cap.data[3],
    ])) + 1;
    let file = std::fs::read(first).map_err(text)?;
    let mut at = 0u64;
    while at < blocks {
        let n = (blocks - at).min(256);
        let mut cdb = [0u8; 10];
        cdb[0] = 0x28;
        cdb[2..6].copy_from_slice(&u32::try_from(at).map_err(text)?.to_be_bytes());
        cdb[7..9].copy_from_slice(&u16::try_from(n).map_err(text)?.to_be_bytes());
        let read = p.ask(0, &cdb, &[], u32::try_from(n * 2048).map_err(text)?)?;
        let from = usize::try_from(at * 2048).map_err(text)?;
        if read.data != file[from..from + read.data.len()] || read.data.len() as u64 != n * 2048 {
            return Err(format!(
                "blocks {at} to {} read otherwise than the file",
                at + n
            ));
        }
        at += n;
    }
    lines.push(format!(
        "{blocks} blocks of 2048 bytes read through the drive, the same as the image"
    ));
    let write = p.ask(0, &[0x2A, 0, 0, 0, 0, 0, 0, 0, 1, 0], &[0u8; 2048], 0)?;
    if (write.status, write.sense.0) != (STATUS_CHECK, 0x07) {
        return Err(format!(
            "a write was answered {}/{:#x}",
            write.status, write.sense.0
        ));
    }
    lines.push("a write refused: data protect".into());
    for bad in [
        &[0xFFu8; 16][..],
        &[0x28, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0xFF, 0xFF, 0][..],
        &[0x43, 0, 0, 0, 0, 0, 0, 0, 0, 0xC0][..],
    ] {
        let a = p.ask(0, bad, &[], 512)?;
        if a.status != STATUS_CHECK {
            return Err(format!("a malformed command {bad:02x?} was answered good"));
        }
    }
    lines.push("malformed commands refused with a check condition".into());
    // Prevented: the host's unload fails unless forced, the guest's
    // eject is refused.
    p.ask(0, &[0x1E, 0, 0, 0, 1, 0], &[], 0)?;
    if drive.unload(false).is_ok() {
        return Err("a prevented medium was unloaded without force".into());
    }
    let eject = p.ask(0, &[0x1B, 0, 0, 0, 2, 0], &[], 0)?;
    if eject.status != STATUS_CHECK {
        return Err("a prevented eject was done".into());
    }
    p.ask(0, &[0x1E, 0, 0, 0, 0, 0], &[], 0)?;
    p.ask(0, &[0x1B, 0, 0, 0, 2, 0], &[], 0)?;
    if drive.loaded().is_some() {
        return Err("the guest's eject left the medium".into());
    }
    lines.push("removal prevented: the host's unload refused, the guest's eject refused; allowed, the guest's eject unloads".into());
    drive.load(Box::new(open(second)?), "SECOND")?;
    let changed = p.ask(0, &[0, 0, 0, 0, 0, 0], &[], 0)?;
    if (changed.status, changed.sense) != (STATUS_CHECK, (6, 0x28)) {
        return Err("the new medium was not told by a unit attention".into());
    }
    lines.push("another image loaded: told by a unit attention".into());
    Ok(lines)
}

/// A tape volume written and read through a tape drive: blocks and
/// filemarks, the end of data, the early warning and the end of the
/// volume, a write-protected volume refusing writes.
pub fn tape(volume: &Path) -> Result<Vec<String>, String> {
    use devices::virtio::scsi::tape::Tape;
    let rw = || {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(volume)
            .map_err(|e| format!("{}: {e}", volume.display()))
    };
    let drive = std::sync::Arc::new(Tape::new("PROBE-TAP").ok_or("a drive")?);
    drive
        .mount(Box::new(rw()?), "PROBE", 16 << 20, false)
        .map_err(text)?;
    let mut p = Probe::of(drive.clone())?;
    let mut lines = Vec::new();
    while p.ask(0, &[0, 0, 0, 0, 0, 0], &[], 0)?.status == STATUS_CHECK {}
    let label = [0xE5u8; 80];
    let block = vec![0x42u8; 65536];
    let w = |n: u32| {
        let b = n.to_be_bytes();
        [0x0A, 0, b[1], b[2], b[3], 0]
    };
    p.ask(0, &w(80), &label, 0)?;
    p.ask(0, &[0x10, 0, 0, 0, 1, 0], &[], 0)?;
    p.ask(0, &w(65536), &block, 0)?;
    p.ask(0, &[0x10, 0, 0, 0, 2, 0], &[], 0)?;
    p.ask(0, &[0x01, 0, 0, 0, 0, 0], &[], 0)?;
    let r = |n: u32| {
        let b = n.to_be_bytes();
        [0x08, 2, b[1], b[2], b[3], 0]
    };
    let first = p.ask(0, &r(262_144), &[], 262_144)?;
    if first.data != label {
        return Err("the label read back otherwise".into());
    }
    let mark = p.ask(0, &r(65536), &[], 65536)?;
    if (mark.status, mark.marks & 0x80) != (STATUS_CHECK, 0x80) {
        return Err("the filemark was not reported".into());
    }
    let data = p.ask(0, &r(262_144), &[], 262_144)?;
    if data.data != block {
        return Err("the data block read back otherwise".into());
    }
    p.ask(0, &r(16), &[], 16)?;
    p.ask(0, &r(16), &[], 16)?;
    let end = p.ask(0, &r(16), &[], 16)?;
    if (end.status, end.sense.0) != (STATUS_CHECK, 0x08) {
        return Err(format!(
            "the end of data was answered {}/{:#x}",
            end.status, end.sense.0
        ));
    }
    lines.push("a label, a filemark, a 64 KiB block and two filemarks written and read back; the filemark reported, the end of data a blank check".into());
    // Written until the end: the early warning, then the end.
    let mut warned = false;
    let mut overflowed = false;
    for _ in 0..1000 {
        let a = p.ask(0, &w(65536), &block, 0)?;
        if a.status == STATUS_CHECK && a.sense.0 == 0 && a.marks & 0x40 != 0 {
            warned = true;
        }
        if a.status == STATUS_CHECK && a.sense.0 == 0x0D {
            overflowed = true;
            break;
        }
    }
    if !warned || !overflowed {
        return Err(format!(
            "written to the end: early warning {warned}, end {overflowed}"
        ));
    }
    lines.push("written to the end of a 16 MiB volume: the early warning first, then the end of the volume with nothing written".into());
    drive.unmount(true).map_err(text)?;
    let ro = std::fs::File::open(volume).map_err(text)?;
    drive
        .mount(Box::new(ro), "PROBE", 16 << 20, true)
        .map_err(text)?;
    while p.ask(0, &[0, 0, 0, 0, 0, 0], &[], 0)?.status == STATUS_CHECK {}
    let refused = p.ask(0, &w(80), &label, 0)?;
    if (refused.status, refused.sense.0) != (STATUS_CHECK, 0x07) {
        return Err("a write-protected volume took a write".into());
    }
    lines.push("write-protected: a write refused with data protect".into());
    Ok(lines)
}
