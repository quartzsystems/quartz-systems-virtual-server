// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A disk LUN: the commands of [`super::cdb`] carried out on a
//! backing store. Every range is checked against the LUN before the
//! host is asked for anything, and data moves through one buffer of a
//! fixed size however much the guest asks for.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use super::backing::{Backing, Bounce};
use super::cdb::{Command, Sense};

/// Bytes moved at a time.
pub const CHUNK_BYTES: usize = 1 << 20;
/// The most blocks one WRITE SAME may name.
pub const MAX_WRITE_SAME_BLOCKS: u32 = 1 << 23;
/// The most descriptors one UNMAP may carry.
pub const MAX_UNMAP_DESCRIPTORS: usize = 64;
/// The most blocks one read or write may name: what the transport's
/// `max_sectors` says.
pub const MAX_TRANSFER_BLOCKS: u32 = 1 << 16;
/// A serial number's most characters.
pub const SERIAL_MAX: usize = 20;

/// The vendor identification, eight characters.
const VENDOR: &[u8; 8] = b"QUARTZ  ";
/// The product identification, sixteen characters.
const PRODUCT: &[u8; 16] = b"VIRTUAL DISK    ";
/// The product revision, four characters.
const REVISION: &[u8; 4] = b"0100";

/// How a command went wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// A check condition with sense data.
    Check(Sense),
    /// The guest's buffers do not hold the transfer the command names.
    Overrun,
}

impl From<Sense> for Failure {
    fn from(sense: Sense) -> Self {
        Failure::Check(sense)
    }
}

/// What a LUN was asked for since it was made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Reads carried out.
    pub reads: u64,
    /// Writes carried out.
    pub writes: u64,
    /// Flushes of the backing store: SYNCHRONIZE CACHE and forced writes.
    pub flushes: u64,
    /// Commands refused with sense data or as an overrun.
    pub refused: u64,
}

#[derive(Default)]
struct Counters {
    reads: AtomicU64,
    writes: AtomicU64,
    flushes: AtomicU64,
    refused: AtomicU64,
}

/// One disk LUN.
pub struct Lun {
    backing: Box<dyn Backing>,
    serial: String,
    blocks: u64,
    block_bytes: u32,
    counters: Counters,
}

/// Write as much of `bytes` as the initiator made room for.
fn give(data_in: &mut dyn Write, bytes: &[u8], room: usize) -> usize {
    let mut given = 0;
    let wanted = &bytes[..bytes.len().min(room)];
    while given < wanted.len() {
        match data_in.write(&wanted[given..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => given += n,
        }
    }
    given
}

impl Lun {
    /// A LUN over `backing` with the serial number `serial`: 1 to 20
    /// characters, letters, digits, `-` and `_`.
    pub fn new(backing: Box<dyn Backing>, serial: &str) -> Option<Self> {
        let valid = (1..=SERIAL_MAX).contains(&serial.len())
            && serial
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        let block_bytes = backing.block_bytes();
        let blocks = backing.bytes() / u64::from(block_bytes.max(1));
        (valid && block_bytes >= 512 && blocks > 0).then(|| Self {
            backing,
            serial: serial.to_string(),
            blocks,
            block_bytes,
            counters: Counters::default(),
        })
    }

    /// Blocks of the LUN.
    pub fn blocks(&self) -> u64 {
        self.blocks
    }

    /// What it was asked for so far.
    pub fn counts(&self) -> Counts {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        Counts {
            reads: get(&self.counters.reads),
            writes: get(&self.counters.writes),
            flushes: get(&self.counters.flushes),
            refused: get(&self.counters.refused),
        }
    }

    fn flush(&self) -> Result<(), Sense> {
        self.backing.flush().map_err(|_| Sense::WRITE_ERROR)?;
        self.counters.flushes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// The byte range of `blocks` blocks from `lba`, when they are all
    /// on the LUN.
    fn range(&self, lba: u64, blocks: u64) -> Result<(u64, u64), Sense> {
        let end = lba.checked_add(blocks).ok_or(Sense::LBA_OUT_OF_RANGE)?;
        if end > self.blocks {
            return Err(Sense::LBA_OUT_OF_RANGE);
        }
        let size = u64::from(self.block_bytes);
        Ok((lba * size, blocks * size))
    }

    fn transfer(&self, lba: u64, blocks: u32) -> Result<(u64, u64), Sense> {
        if blocks > MAX_TRANSFER_BLOCKS {
            return Err(Sense::INVALID_FIELD);
        }
        self.range(lba, u64::from(blocks))
    }

    fn inquiry() -> Vec<u8> {
        let mut b = vec![0u8; 36];
        b[2] = 6; // SPC-4
        b[3] = 2; // response data format
        b[4] = 31; // additional length
        b[7] = 0x02; // command queuing
        b[8..16].copy_from_slice(VENDOR);
        b[16..32].copy_from_slice(PRODUCT);
        b[32..36].copy_from_slice(REVISION);
        b
    }

    fn vital_page(&self, page: u8) -> Result<Vec<u8>, Sense> {
        let body: Vec<u8> = match page {
            0x00 => vec![0x00, 0x80, 0x83, 0xB0, 0xB1, 0xB2],
            0x80 => self.serial.as_bytes().to_vec(),
            0x83 => {
                // One designator: the T10 vendor id and the serial number.
                let mut id = VENDOR.to_vec();
                id.extend_from_slice(self.serial.as_bytes());
                #[allow(clippy::cast_possible_truncation)] // 28 at most
                let mut d = vec![0x02, 0x01, 0x00, id.len() as u8];
                d.extend_from_slice(&id);
                d
            }
            0xB0 => {
                let mut b = vec![0u8; 60];
                b[0] = 1; // WSNZ: a WRITE SAME of no blocks is not "to the end"
                b[4..8].copy_from_slice(&MAX_TRANSFER_BLOCKS.to_be_bytes());
                if self.backing.can_discard() {
                    b[16..20].copy_from_slice(&MAX_WRITE_SAME_BLOCKS.to_be_bytes());
                    #[allow(clippy::cast_possible_truncation)]
                    b[20..24].copy_from_slice(&(MAX_UNMAP_DESCRIPTORS as u32).to_be_bytes());
                    b[24..28].copy_from_slice(&1u32.to_be_bytes());
                }
                b[32..40].copy_from_slice(&u64::from(MAX_WRITE_SAME_BLOCKS).to_be_bytes());
                b
            }
            0xB1 => {
                let mut b = vec![0u8; 60];
                b[1] = 1; // not a rotating medium
                b
            }
            0xB2 => {
                let mut b = vec![0u8; 4];
                if self.backing.can_discard() {
                    b[1] = 0x80 | 0x40; // UNMAP, and WRITE SAME (16) with UNMAP
                    if self.backing.discard_zeroes() {
                        b[1] |= 0x04;
                    }
                    b[2] = 2; // thin provisioned
                }
                b
            }
            _ => return Err(Sense::INVALID_FIELD),
        };
        #[allow(clippy::cast_possible_truncation)] // sixty at most
        let mut out = vec![0, page, 0, body.len() as u8];
        out.extend_from_slice(&body);
        Ok(out)
    }

    fn capacity16(&self) -> Vec<u8> {
        let mut b = vec![0u8; 32];
        b[..8].copy_from_slice(&(self.blocks - 1).to_be_bytes());
        b[8..12].copy_from_slice(&self.block_bytes.to_be_bytes());
        if self.backing.can_discard() {
            b[14] = 0x80; // LBPME
            if self.backing.discard_zeroes() {
                b[14] |= 0x40;
            }
        }
        b
    }

    fn mode_sense(
        &self,
        ten: bool,
        no_bd: bool,
        changeable: bool,
        page: u8,
    ) -> Result<Vec<u8>, Sense> {
        if page != 0x08 && page != 0x3F {
            return Err(Sense::INVALID_FIELD);
        }
        let mut body = Vec::new();
        if !no_bd {
            // The short block descriptor: block count (or ones), block length.
            let count = u32::try_from(self.blocks).unwrap_or(u32::MAX);
            body.extend_from_slice(&count.to_be_bytes());
            body.extend_from_slice(&self.block_bytes.to_be_bytes());
        }
        let descriptor = body.len();
        // The caching page: the write cache is on, and cannot be changed.
        let mut caching = [0u8; 20];
        caching[0] = 0x08;
        caching[1] = 0x12;
        if !changeable {
            caching[2] = 0x04;
        }
        body.extend_from_slice(&caching);
        // Device-specific parameter: DPO and FUA are taken.
        let specific = 0x10;
        let mut out = Vec::with_capacity(body.len() + 8);
        #[allow(clippy::cast_possible_truncation)] // 36 at most
        if ten {
            out.extend_from_slice(&(body.len() as u16 + 6).to_be_bytes());
            out.extend_from_slice(&[0, specific, 0, 0]);
            out.extend_from_slice(&(descriptor as u16).to_be_bytes());
        } else {
            out.extend_from_slice(&[body.len() as u8 + 3, 0, specific, descriptor as u8]);
        }
        out.extend_from_slice(&body);
        Ok(out)
    }

    fn read(
        &self,
        lba: u64,
        blocks: u32,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let (mut offset, bytes) = self.transfer(lba, blocks)?;
        let mut left = usize::try_from(bytes).map_err(|_| Sense::INVALID_FIELD)?;
        let total = left;
        while left > 0 {
            let n = left.min(bounce.len());
            let chunk = &mut bounce.bytes()[..n];
            self.backing
                .read_at(chunk, offset)
                .map_err(|_| Sense::READ_ERROR)?;
            data_in.write_all(chunk).map_err(|_| Failure::Overrun)?;
            offset += n as u64;
            left -= n;
        }
        Ok(total)
    }

    fn write(
        &self,
        lba: u64,
        blocks: u32,
        fua: bool,
        data_out: &mut dyn Read,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let (mut offset, bytes) = self.transfer(lba, blocks)?;
        let mut left = usize::try_from(bytes).map_err(|_| Sense::INVALID_FIELD)?;
        while left > 0 {
            let n = left.min(bounce.len());
            let chunk = &mut bounce.bytes()[..n];
            data_out.read_exact(chunk).map_err(|_| Failure::Overrun)?;
            self.backing
                .write_at(chunk, offset)
                .map_err(|_| Sense::WRITE_ERROR)?;
            offset += n as u64;
            left -= n;
        }
        if fua {
            self.flush()?;
        }
        Ok(0)
    }

    fn unmap(&self, length: usize, data_out: &mut dyn Read) -> Result<usize, Failure> {
        if !self.backing.can_discard() {
            return Err(Sense::INVALID_OPCODE.into());
        }
        if length == 0 {
            return Ok(0);
        }
        if !(8..=8 + MAX_UNMAP_DESCRIPTORS * 16).contains(&length) {
            return Err(Sense::PARAMETER_LENGTH.into());
        }
        let mut list = [0u8; 8 + MAX_UNMAP_DESCRIPTORS * 16];
        let list = &mut list[..length];
        data_out.read_exact(list).map_err(|_| Failure::Overrun)?;
        let described = usize::from(u16::from_be_bytes([list[2], list[3]]));
        if described % 16 != 0 || described > length - 8 {
            return Err(Sense::INVALID_PARAMETER.into());
        }
        // Every range is checked before any is given back.
        let mut ranges = Vec::with_capacity(described / 16);
        for d in list[8..8 + described].as_chunks::<16>().0 {
            let lba = u64::from_be_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
            let blocks = u32::from_be_bytes([d[8], d[9], d[10], d[11]]);
            ranges.push(self.range(lba, u64::from(blocks))?);
        }
        for (offset, bytes) in ranges {
            if bytes > 0 {
                self.backing
                    .discard(offset, bytes)
                    .map_err(|_| Sense::WRITE_ERROR)?;
            }
        }
        Ok(0)
    }

    fn write_same(
        &self,
        lba: u64,
        blocks: u32,
        unmap: bool,
        no_data: bool,
        data_out: &mut dyn Read,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        if blocks == 0 || blocks > MAX_WRITE_SAME_BLOCKS {
            return Err(Sense::INVALID_FIELD.into());
        }
        let (mut offset, bytes) = self.range(lba, u64::from(blocks))?;
        let block = self.block_bytes as usize;
        let zeros = if no_data {
            true
        } else {
            let pattern = &mut bounce.bytes()[..block];
            data_out.read_exact(pattern).map_err(|_| Failure::Overrun)?;
            pattern.iter().all(|b| *b == 0)
        };
        if zeros {
            let discarding = unmap && self.backing.can_discard() && self.backing.discard_zeroes();
            let done = if discarding {
                self.backing.discard(offset, bytes)
            } else {
                self.backing.write_zeroes(offset, bytes)
            };
            if done.is_ok() {
                return Ok(0);
            }
            // The host would not: write the zeros out below.
            bounce.bytes()[..block].fill(0);
        }
        // Fill the buffer with the block and write it until done.
        let whole = bounce.len() - bounce.len() % block;
        let buffer = &mut bounce.bytes()[..whole];
        let (pattern, rest) = buffer.split_at_mut(block);
        for copy in rest.chunks_exact_mut(block) {
            copy.copy_from_slice(pattern);
        }
        let mut left = bytes;
        while left > 0 {
            #[allow(clippy::cast_possible_truncation)] // the smaller is a usize
            let n = left.min(whole as u64) as usize;
            self.backing
                .write_at(&buffer[..n], offset)
                .map_err(|_| Sense::WRITE_ERROR)?;
            offset += n as u64;
            left -= n as u64;
        }
        Ok(0)
    }

    /// Carry out `command`. `luns` is how many LUNs the target has,
    /// for REPORT LUNS. The bytes written to `data_in`.
    pub fn execute(
        &self,
        command: Command,
        luns: usize,
        data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let result = self.carry_out(command, luns, data_out, data_in, bounce);
        let counter = match (&result, command) {
            (Err(_), _) => &self.counters.refused,
            (Ok(_), Command::Read { .. }) => &self.counters.reads,
            (Ok(_), Command::Write { .. }) => &self.counters.writes,
            _ => return result,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        result
    }

    /// Note a command refused before it reached the LUN.
    pub fn refused(&self) {
        self.counters.refused.fetch_add(1, Ordering::Relaxed);
    }

    fn carry_out(
        &self,
        command: Command,
        luns: usize,
        data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        match command {
            Command::Nothing => Ok(0),
            Command::RequestSense { room } => {
                // Nothing is ever deferred: there is no sense to report.
                let sense = Sense {
                    key: 0,
                    asc: 0,
                    ascq: 0,
                };
                Ok(give(data_in, &sense.bytes(), room))
            }
            Command::Inquiry { page: None, room } => Ok(give(data_in, &Self::inquiry(), room)),
            Command::Inquiry {
                page: Some(page),
                room,
            } => Ok(give(data_in, &self.vital_page(page)?, room)),
            Command::ReportLuns { room } => {
                let mut b = Vec::with_capacity(8 + luns * 8);
                #[allow(clippy::cast_possible_truncation)] // 256 LUNs at most
                b.extend_from_slice(&(luns as u32 * 8).to_be_bytes());
                b.extend_from_slice(&[0; 4]);
                for lun in 0..luns {
                    #[allow(clippy::cast_possible_truncation)]
                    b.extend_from_slice(&[(lun >> 8) as u8 & 0x3F, lun as u8, 0, 0, 0, 0, 0, 0]);
                }
                Ok(give(data_in, &b, room))
            }
            Command::ReadCapacity10 => {
                let last = u32::try_from(self.blocks - 1).unwrap_or(u32::MAX);
                let mut b = last.to_be_bytes().to_vec();
                b.extend_from_slice(&self.block_bytes.to_be_bytes());
                Ok(give(data_in, &b, 8))
            }
            Command::ReadCapacity16 { room } => Ok(give(data_in, &self.capacity16(), room)),
            Command::Read { lba, blocks } => self.read(lba, blocks, data_in, bounce),
            Command::Write { lba, blocks, fua } => self.write(lba, blocks, fua, data_out, bounce),
            Command::Verify { lba, blocks } => {
                self.transfer(lba, blocks)?;
                Ok(0)
            }
            Command::SynchronizeCache => {
                self.flush()?;
                Ok(0)
            }
            Command::ModeSense {
                ten,
                no_block_descriptor,
                changeable,
                page,
                room,
            } => {
                let data = self.mode_sense(ten, no_block_descriptor, changeable, page)?;
                Ok(give(data_in, &data, room))
            }
            Command::Unmap { length } => self.unmap(length, data_out),
            Command::WriteSame {
                lba,
                blocks,
                unmap,
                no_data,
            } => self.write_same(lba, blocks, unmap, no_data, data_out, bounce),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::backing::memory::MemoryStore;
    use super::super::cdb::decode;
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    /// A store the test keeps a hold of.
    struct Held(Arc<MemoryStore>);

    impl Backing for Held {
        fn bytes(&self) -> u64 {
            self.0.bytes()
        }
        fn block_bytes(&self) -> u32 {
            self.0.block_bytes()
        }
        fn read_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
            self.0.read_at(buf, offset)
        }
        fn write_at(&self, buf: &[u8], offset: u64) -> std::io::Result<()> {
            self.0.write_at(buf, offset)
        }
        fn flush(&self) -> std::io::Result<()> {
            self.0.flush()
        }
        fn can_discard(&self) -> bool {
            self.0.can_discard()
        }
        fn discard_zeroes(&self) -> bool {
            self.0.discard_zeroes()
        }
        fn discard(&self, offset: u64, bytes: u64) -> std::io::Result<()> {
            self.0.discard(offset, bytes)
        }
        fn write_zeroes(&self, offset: u64, bytes: u64) -> std::io::Result<()> {
            self.0.write_zeroes(offset, bytes)
        }
    }

    fn lun(discardable: bool) -> (Lun, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::new(64 * 512, discardable));
        (
            Lun::new(Box::new(Held(store.clone())), "QSVS-LUN_0").unwrap(),
            store,
        )
    }

    fn run(lun: &Lun, cdb: &[u8], out: &[u8]) -> Result<Vec<u8>, Failure> {
        let mut data_in = Vec::new();
        let mut bounce = Bounce::new(4096);
        let command = decode(cdb)?;
        let n = lun.execute(command, 2, &mut &out[..], &mut data_in, &mut bounce)?;
        assert_eq!(n, data_in.len());
        Ok(data_in)
    }

    fn rw10(opcode: u8, lba: u32, blocks: u16) -> [u8; 10] {
        let mut c = [0u8; 10];
        c[0] = opcode;
        c[2..6].copy_from_slice(&lba.to_be_bytes());
        c[7..9].copy_from_slice(&blocks.to_be_bytes());
        c
    }

    #[test]
    fn what_is_written_is_read_back_and_is_in_the_store() {
        let (lun, store) = lun(false);
        let data: Vec<u8> = (0..3 * 512u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(run(&lun, &rw10(0x2A, 5, 3), &data), Ok(vec![]));
        assert_eq!(store.data.lock().unwrap()[5 * 512..8 * 512], data[..]);
        assert_eq!(run(&lun, &rw10(0x28, 5, 3), &[]), Ok(data.clone()));
        // A transfer larger than the buffer moves in pieces.
        let big: Vec<u8> = (0..20 * 512u32).map(|i| (i % 13) as u8).collect();
        assert_eq!(run(&lun, &rw10(0x2A, 40, 20), &big), Ok(vec![]));
        assert_eq!(run(&lun, &rw10(0x28, 40, 20), &[]), Ok(big));
        assert_eq!(store.flushes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_flush_and_a_forced_write_reach_the_store() {
        let (lun, store) = lun(false);
        assert_eq!(
            run(&lun, &[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[]),
            Ok(vec![])
        );
        assert_eq!(store.flushes.load(Ordering::SeqCst), 1);
        let mut fua = rw10(0x2A, 0, 1);
        fua[1] = 0x08;
        assert_eq!(run(&lun, &fua, &[7; 512]), Ok(vec![]));
        assert_eq!(store.flushes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn requests_beyond_the_lun_never_reach_the_store() {
        let (lun, store) = lun(true);
        let out_of_range = Err(Failure::Check(Sense::LBA_OUT_OF_RANGE));
        assert_eq!(run(&lun, &rw10(0x28, 64, 1), &[]), out_of_range);
        assert_eq!(run(&lun, &rw10(0x28, 63, 2), &[]), out_of_range);
        assert_eq!(
            run(&lun, &rw10(0x2A, 0xFFFF_FFFF, 1), &[0; 512]),
            out_of_range
        );
        assert_eq!(run(&lun, &rw10(0x2F, 60, 5), &[]), out_of_range);
        let mut read16 = [0u8; 16];
        read16[0] = 0x88;
        read16[2..10].copy_from_slice(&u64::MAX.to_be_bytes());
        read16[13] = 2;
        assert_eq!(run(&lun, &read16, &[]), out_of_range);
        read16[2..10].copy_from_slice(&0u64.to_be_bytes());
        read16[10..14].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            run(&lun, &read16, &[]),
            Err(Failure::Check(Sense::INVALID_FIELD))
        );
        // The last block is on the LUN; no blocks at its end is nothing.
        assert_eq!(run(&lun, &rw10(0x28, 63, 1), &[]).map(|d| d.len()), Ok(512));
        assert_eq!(run(&lun, &rw10(0x28, 64, 0), &[]), Ok(vec![]));
        // A write with too little data is an overrun and writes nothing whole.
        assert_eq!(
            run(&lun, &rw10(0x2A, 0, 2), &[1; 600]),
            Err(Failure::Overrun)
        );
        assert!(store.discards.lock().unwrap().is_empty());
    }

    #[test]
    fn the_enquiries_describe_the_lun() {
        let (lun, _) = lun(true);
        let inquiry = run(&lun, &[0x12, 0, 0, 0, 96, 0], &[]).unwrap();
        assert_eq!(inquiry.len(), 36);
        assert_eq!(&inquiry[8..16], b"QUARTZ  ");
        // The initiator's room bounds the answer.
        assert_eq!(run(&lun, &[0x12, 0, 0, 0, 5, 0], &[]).unwrap().len(), 5);
        let serial = run(&lun, &[0x12, 1, 0x80, 0, 255, 0], &[]).unwrap();
        assert_eq!(&serial[4..], b"QSVS-LUN_0");
        assert_eq!(serial[3] as usize, 10);
        let id = run(&lun, &[0x12, 1, 0x83, 0, 255, 0], &[]).unwrap();
        assert_eq!(&id[8..], b"QUARTZ  QSVS-LUN_0");
        let pages = run(&lun, &[0x12, 1, 0x00, 0, 255, 0], &[]).unwrap();
        assert_eq!(pages[4..], [0x00, 0x80, 0x83, 0xB0, 0xB1, 0xB2]);
        assert_eq!(
            run(&lun, &[0x12, 1, 0x99, 0, 255, 0], &[]),
            Err(Failure::Check(Sense::INVALID_FIELD))
        );
        let capacity = run(&lun, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[]).unwrap();
        assert_eq!(capacity, [0, 0, 0, 63, 0, 0, 2, 0]);
        let mut c16 = [0u8; 16];
        c16[0] = 0x9E;
        c16[1] = 0x10;
        c16[13] = 32;
        let capacity = run(&lun, &c16, &[]).unwrap();
        assert_eq!(capacity[7], 63);
        assert_eq!(capacity[14], 0xC0);
        let luns = run(&lun, &[0xA0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0], &[]).unwrap();
        assert_eq!(luns[..4], [0, 0, 0, 16]);
        assert_eq!(
            luns[8..24],
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]
        );
        // The write cache is on, so the guest sends its flushes.
        let mode = run(&lun, &[0x1A, 0, 0x08, 0, 255, 0], &[]).unwrap();
        assert_eq!(mode[0] as usize, mode.len() - 1);
        assert_eq!((mode[2], mode[3]), (0x10, 8));
        assert_eq!((mode[12], mode[14]), (0x08, 0x04));
        let mode10 = run(&lun, &[0x5A, 0x08, 0x3F, 0, 0, 0, 0, 0, 255, 0], &[]).unwrap();
        assert_eq!(
            u16::from_be_bytes([mode10[0], mode10[1]]) as usize,
            mode10.len() - 2
        );
        assert_eq!((mode10[8], mode10[10]), (0x08, 0x04));
        assert_eq!(run(&lun, &[0x03, 0, 0, 0, 18, 0], &[]).unwrap()[0], 0x70);
    }

    #[test]
    fn unmap_and_write_same_keep_to_the_lun() {
        let (lun, store) = lun(true);
        store.data.lock().unwrap().fill(0xEE);
        let mut list = vec![0, 22, 0, 16, 0, 0, 0, 0];
        list.extend_from_slice(&4u64.to_be_bytes());
        list.extend_from_slice(&2u32.to_be_bytes());
        list.extend_from_slice(&[0; 4]);
        let unmap = [0x42, 0, 0, 0, 0, 0, 0, 0, 24, 0];
        assert_eq!(run(&lun, &unmap, &list), Ok(vec![]));
        assert_eq!(*store.discards.lock().unwrap(), [(4 * 512, 2 * 512)]);
        assert_eq!(store.data.lock().unwrap()[4 * 512..6 * 512], [0u8; 1024]);
        // A range off the end unmaps nothing, not even the good range before it.
        let mut two = vec![0, 38, 0, 32, 0, 0, 0, 0];
        two.extend_from_slice(&0u64.to_be_bytes());
        two.extend_from_slice(&1u32.to_be_bytes());
        two.extend_from_slice(&[0; 4]);
        two.extend_from_slice(&63u64.to_be_bytes());
        two.extend_from_slice(&2u32.to_be_bytes());
        two.extend_from_slice(&[0; 4]);
        let unmap = [0x42, 0, 0, 0, 0, 0, 0, 0, 40, 0];
        assert_eq!(
            run(&lun, &unmap, &two),
            Err(Failure::Check(Sense::LBA_OUT_OF_RANGE))
        );
        assert_eq!(store.discards.lock().unwrap().len(), 1);
        // A list that lies about its length.
        let unmap = [0x42, 0, 0, 0, 0, 0, 0, 0, 12, 0];
        assert_eq!(
            run(&lun, &unmap, &[0, 30, 0, 32, 0, 0, 0, 0, 1, 2, 3, 4]),
            Err(Failure::Check(Sense::INVALID_PARAMETER))
        );
        let unmap = [0x42, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0];
        assert_eq!(
            run(&lun, &unmap, &[]),
            Err(Failure::Check(Sense::PARAMETER_LENGTH))
        );

        // WRITE SAME with a pattern, then with zeros and UNMAP.
        let mut same = [0u8; 16];
        same[0] = 0x93;
        same[9] = 10;
        same[13] = 12;
        assert_eq!(run(&lun, &same, &[0xAB; 512]), Ok(vec![]));
        assert_eq!(
            store.data.lock().unwrap()[10 * 512..22 * 512],
            [0xAB; 12 * 512]
        );
        assert_eq!(store.data.lock().unwrap()[22 * 512], 0xEE);
        same[1] = 0x08;
        assert_eq!(run(&lun, &same, &[0; 512]), Ok(vec![]));
        assert_eq!(
            store.discards.lock().unwrap().last(),
            Some(&(10 * 512, 12 * 512))
        );
        same[9] = 60;
        assert_eq!(
            run(&lun, &same, &[0; 512]),
            Err(Failure::Check(Sense::LBA_OUT_OF_RANGE))
        );
        same[13] = 0;
        assert_eq!(
            run(&lun, &same, &[0; 512]),
            Err(Failure::Check(Sense::INVALID_FIELD))
        );
    }

    #[test]
    fn a_store_that_cannot_discard_says_so() {
        let (lun, _) = lun(false);
        let unmap = [0x42, 0, 0, 0, 0, 0, 0, 0, 24, 0];
        assert_eq!(
            run(&lun, &unmap, &[0; 24]),
            Err(Failure::Check(Sense::INVALID_OPCODE))
        );
        let lbp = run(&lun, &[0x12, 1, 0xB2, 0, 255, 0], &[]).unwrap();
        assert_eq!(lbp[5], 0);
        let limits = run(&lun, &[0x12, 1, 0xB0, 0, 255, 0], &[]).unwrap();
        assert_eq!(limits[4 + 20..4 + 24], [0; 4]);
    }

    #[test]
    fn a_serial_number_is_checked() {
        let store = || Box::new(MemoryStore::new(4096, false));
        assert!(Lun::new(store(), "").is_none());
        assert!(Lun::new(store(), "has space").is_none());
        assert!(Lun::new(store(), &"X".repeat(21)).is_none());
        assert!(Lun::new(Box::new(MemoryStore::new(100, false)), "OK").is_none());
        assert_eq!(Lun::new(store(), "OK").unwrap().blocks(), 8);
    }
}
