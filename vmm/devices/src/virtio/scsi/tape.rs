// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A virtual tape drive: a sequential-access device whose volume is an
//! image of the media library (`stg/tape`). It answers what Linux's
//! `st` driver sends, which is all QS/400 sends to it
//! (`docs/QS400_GUEST.md`): INQUIRY, TEST UNIT READY, REQUEST SENSE,
//! READ BLOCK LIMITS, MODE SENSE and MODE SELECT (fixed and variable
//! blocks), READ (6) and WRITE (6), WRITE FILEMARKS, SPACE (blocks,
//! filemarks, end of data), REWIND, LOCATE, READ POSITION, LOAD UNLOAD,
//! ERASE, PREVENT ALLOW MEDIUM REMOVAL, REPORT DENSITY SUPPORT and
//! REPORT LUNS.
//!
//! The residue and the filemark, end-of-medium and end-of-data
//! conditions are reported in the sense data as the standard has them:
//! a filemark read or spaced to is NO SENSE with the filemark bit; the
//! end of data BLANK CHECK; the early warning NO SENSE with the
//! end-of-medium bit, the block written; the end of the volume VOLUME
//! OVERFLOW, nothing written; a block of another length than asked for
//! the incorrect-length bit; the information field is what was not
//! done of the count.
//!
//! A write is durable when the guest writes a filemark (or none, which
//! is how it asks for a flush), and when the volume is unmounted.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use ::tape::medium::{Edge, MAX_BLOCK, Medium, Read as TapeRead, Stop, Store, TapeError};

use super::backing::Bounce;
use super::cdb::Sense;
use super::lun::{Counts, Failure};
use super::unit::{Unit, give, identity_page, inquiry, report_luns, valid_serial};

const PRODUCT: &[u8; 16] = b"VIRTUAL TAPE    ";
/// The density code the drive reports.
const DENSITY: u8 = 0x00;

const NO_SENSE: u8 = 0x00;
const NOT_READY: u8 = 0x02;
const MEDIUM_ERROR: u8 = 0x03;
const ILLEGAL_REQUEST: u8 = 0x05;
const UNIT_ATTENTION: u8 = 0x06;
const DATA_PROTECT: u8 = 0x07;
const BLANK_CHECK: u8 = 0x08;
const VOLUME_OVERFLOW: u8 = 0x0D;

const NO_MEDIUM: Sense = Sense::new(NOT_READY, 0x3A, 0x00);
const MEDIUM_CHANGED: Sense = Sense::new(UNIT_ATTENTION, 0x28, 0x00);
const POWER_ON: Sense = Sense::new(UNIT_ATTENTION, 0x29, 0x00);
const WRITE_PROTECTED: Sense = Sense::new(DATA_PROTECT, 0x27, 0x00);
const REMOVAL_PREVENTED: Sense = Sense::new(ILLEGAL_REQUEST, 0x53, 0x02);
const FILEMARK: Sense = Sense::new(NO_SENSE, 0x00, 0x01);
const END_OF_DATA: Sense = Sense::new(BLANK_CHECK, 0x00, 0x05);
const BEGINNING: Sense = Sense::new(NO_SENSE, 0x00, 0x04);
const EARLY_WARNING: Sense = Sense::new(NO_SENSE, 0x00, 0x02);
const OVERFLOW: Sense = Sense::new(VOLUME_OVERFLOW, 0x00, 0x02);
const LENGTH: Sense = Sense::new(NO_SENSE, 0x00, 0x00);
const WRITE_ERROR: Sense = Sense::new(MEDIUM_ERROR, 0x0C, 0x00);
const READ_ERROR: Sense = Sense::new(MEDIUM_ERROR, 0x11, 0x00);

/// What the drive told the host of its own accord.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The guest unloaded the volume, named here: it is unmounted.
    Unloaded(String),
}

/// Why the host's unmount was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UnmountError {
    /// No volume is mounted.
    #[error("no volume is mounted")]
    Empty,
    /// The guest has prevented the volume's removal.
    #[error("the partition has prevented the volume's removal")]
    Prevented,
    /// The volume's last writes could not be made durable.
    #[error("the volume's writes could not be made durable")]
    NotDurable,
}

struct Mounted {
    medium: Medium<Box<dyn Store>>,
    name: String,
}

struct State {
    mounted: Option<Mounted>,
    /// The fixed block length; 0 for variable blocks.
    block: u32,
    prevented: bool,
    attention: Option<Sense>,
    last: Option<Sense>,
    notices: Vec<Notice>,
}

/// A virtual tape drive.
pub struct Tape {
    serial: String,
    state: Mutex<State>,
    reads: AtomicU64,
    writes: AtomicU64,
    flushes: AtomicU64,
    refused: AtomicU64,
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be24(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([0, b[at], b[at + 1], b[at + 2]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A 24-bit two's complement count.
fn signed24(b: &[u8], at: usize) -> i64 {
    let v = i64::from(be24(b, at));
    if v & 0x80_0000 != 0 {
        v - 0x100_0000
    } else {
        v
    }
}

/// What a space or locate that stopped short reports.
fn stopped(stop: Stop) -> Sense {
    let residue = u32::try_from(stop.residue).unwrap_or(u32::MAX);
    match stop.edge {
        Edge::Filemark => FILEMARK.with(Sense::FILEMARK, residue),
        Edge::EndOfData => END_OF_DATA.with(0, residue),
        Edge::Beginning => BEGINNING.with(Sense::EOM, residue),
    }
}

fn io_failure(e: &TapeError, writing: bool) -> Sense {
    match e {
        TapeError::WriteProtected => WRITE_PROTECTED,
        TapeError::BlockSize(_) => Sense::INVALID_FIELD,
        TapeError::Full | TapeError::TooManyObjects => OVERFLOW.with(Sense::EOM, 0),
        TapeError::Io(_) if writing => WRITE_ERROR,
        TapeError::Io(_) => READ_ERROR,
    }
}

impl Tape {
    /// An empty drive with the serial number `serial`, in variable
    /// block mode.
    pub fn new(serial: &str) -> Option<Self> {
        valid_serial(serial).then(|| Self {
            serial: serial.to_string(),
            state: Mutex::new(State {
                mounted: None,
                block: 0,
                prevented: false,
                attention: Some(POWER_ON),
                last: None,
                notices: Vec::new(),
            }),
            reads: AtomicU64::new(0),
            writes: AtomicU64::new(0),
            flushes: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Mount the volume in `store`, named `name`, of `capacity` bytes,
    /// write-protected or not. A volume already mounted is unmounted
    /// first, its writes made durable.
    pub fn mount(
        &self,
        store: Box<dyn Store>,
        name: &str,
        capacity: u64,
        protected: bool,
    ) -> Result<(), TapeError> {
        let medium = Medium::mount(store, capacity, protected)?;
        let mut s = self.lock();
        if let Some(old) = s.mounted.take() {
            let _ = old.medium.sync();
        }
        s.mounted = Some(Mounted {
            medium,
            name: name.to_string(),
        });
        s.attention = Some(MEDIUM_CHANGED);
        Ok(())
    }

    /// Unmount the volume, its writes made durable; when the guest
    /// prevented it, only `forced`. The name of the volume.
    pub fn unmount(&self, forced: bool) -> Result<String, UnmountError> {
        let mut s = self.lock();
        let Some(m) = &s.mounted else {
            return Err(UnmountError::Empty);
        };
        if s.prevented && !forced {
            return Err(UnmountError::Prevented);
        }
        m.medium.sync().map_err(|_| UnmountError::NotDurable)?;
        let name = s.mounted.take().map(|m| m.name).unwrap_or_default();
        s.prevented = false;
        s.attention = Some(MEDIUM_CHANGED);
        Ok(name)
    }

    /// The name of the volume mounted.
    pub fn mounted(&self) -> Option<String> {
        self.lock().mounted.as_ref().map(|m| m.name.clone())
    }

    /// Whether the guest has prevented the volume's removal.
    pub fn prevented(&self) -> bool {
        self.lock().prevented
    }

    /// Make the mounted volume's writes durable: at the partition's end.
    pub fn sync(&self) -> bool {
        self.lock()
            .mounted
            .as_ref()
            .is_none_or(|m| m.medium.sync().is_ok())
    }

    /// What the drive did of its own accord since the last look.
    pub fn notices(&self) -> Vec<Notice> {
        std::mem::take(&mut self.lock().notices)
    }

    fn medium(s: &mut State) -> Result<&mut Medium<Box<dyn Store>>, Sense> {
        s.mounted.as_mut().map(|m| &mut m.medium).ok_or(NO_MEDIUM)
    }

    /// The mode parameter header's device-specific byte: write
    /// protection and buffered mode.
    fn device_specific(s: &State) -> u8 {
        let protected = s.mounted.as_ref().is_some_and(|m| m.medium.protected());
        0x10 | if protected { 0x80 } else { 0 }
    }

    fn mode_pages(page: u8) -> Result<Vec<u8>, Sense> {
        // The data compression page (off, and not to be turned on) and
        // the device configuration page.
        let compression = {
            let mut p = vec![0u8; 16];
            p[0] = 0x0F;
            p[1] = 14;
            p
        };
        let configuration = {
            let mut p = vec![0u8; 16];
            p[0] = 0x10;
            p[1] = 14;
            p[8] = 0x40; // block identifiers supported
            p[10] = 0x18; // report early warning, synchronize at early warning
            p
        };
        Ok(match page {
            0x00 => Vec::new(),
            0x0F => compression,
            0x10 => configuration,
            0x3F => [compression, configuration].concat(),
            _ => return Err(Sense::INVALID_FIELD),
        })
    }

    fn mode_sense(s: &State, ten: bool, dbd: bool, page: u8) -> Result<Vec<u8>, Sense> {
        let pages = Self::mode_pages(page)?;
        let mut descriptor = Vec::new();
        if !dbd {
            descriptor.push(DENSITY);
            descriptor.extend_from_slice(&[0, 0, 0, 0]);
            descriptor.extend_from_slice(&s.block.to_be_bytes()[1..]);
        }
        let specific = Self::device_specific(s);
        let mut out = Vec::new();
        #[allow(clippy::cast_possible_truncation)] // under 64 bytes
        if ten {
            let len = (6 + descriptor.len() + pages.len()) as u16;
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(&[0, specific, 0, 0]);
            out.extend_from_slice(&(descriptor.len() as u16).to_be_bytes());
        } else {
            let len = (3 + descriptor.len() + pages.len()) as u8;
            out.extend_from_slice(&[len, 0, specific, descriptor.len() as u8]);
        }
        out.extend_from_slice(&descriptor);
        out.extend_from_slice(&pages);
        Ok(out)
    }

    /// MODE SELECT: the block length of the descriptor, when there is
    /// one; the pages are taken as they are.
    fn mode_select(
        s: &mut State,
        ten: bool,
        length: usize,
        data_out: &mut dyn Read,
    ) -> Result<usize, Failure> {
        let header = if ten { 8 } else { 4 };
        if length == 0 {
            return Ok(0);
        }
        if !(header..=256).contains(&length) {
            return Err(Sense::PARAMETER_LENGTH.into());
        }
        let mut list = [0u8; 256];
        let list = &mut list[..length];
        data_out.read_exact(list).map_err(|_| Failure::Overrun)?;
        let bdl = if ten {
            usize::from(be16(list, 6))
        } else {
            usize::from(list[3])
        };
        if bdl > 0 {
            if bdl != 8 || length < header + 8 {
                return Err(Sense::INVALID_PARAMETER.into());
            }
            let block = be24(list, header + 5);
            if block as usize > MAX_BLOCK {
                return Err(Sense::INVALID_PARAMETER.into());
            }
            s.block = block;
        }
        Ok(0)
    }

    fn read_block_limits() -> [u8; 6] {
        let max = u32::try_from(MAX_BLOCK).unwrap_or(0).to_be_bytes();
        [0, max[1], max[2], max[3], 0, 1]
    }

    fn read_position(s: &mut State, form: u8) -> Result<Vec<u8>, Sense> {
        if form != 0 {
            return Err(Sense::INVALID_FIELD);
        }
        let m = Self::medium(s)?;
        let mut b = vec![0u8; 20];
        if m.at_beginning() {
            b[0] |= 0x80;
        }
        if m.past_warning().unwrap_or(false) {
            b[0] |= 0x40;
        }
        let at = u32::try_from(m.position()).unwrap_or(u32::MAX);
        b[4..8].copy_from_slice(&at.to_be_bytes());
        b[8..12].copy_from_slice(&at.to_be_bytes());
        Ok(b)
    }

    fn density_support(s: &State) -> Vec<u8> {
        let mut d = vec![0u8; 52];
        d[0] = DENSITY;
        d[1] = DENSITY;
        d[2] = 0xA0; // writable, the default
        let mib = s.mounted.as_ref().map_or(0, |m| m.medium.capacity() >> 20);
        d[12..16].copy_from_slice(&u32::try_from(mib).unwrap_or(u32::MAX).to_be_bytes());
        d[16..24].copy_from_slice(b"QUARTZ  ");
        d[24..32].copy_from_slice(b"VIRTUAL ");
        d[32..52].copy_from_slice(b"QS/VS virtual tape  ");
        let mut out = vec![0, 54, 0, 0];
        out.extend_from_slice(&d);
        out
    }

    /// READ (6): `count` bytes of one block, or `count` blocks of the
    /// fixed length.
    fn read(
        &self,
        s: &mut State,
        fixed: bool,
        sili: bool,
        count: u32,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        if fixed != (s.block != 0) || (fixed && sili) {
            return Err(Sense::INVALID_FIELD.into());
        }
        let block = s.block as usize;
        let medium = Self::medium(s)?;
        if count == 0 {
            return Ok(0);
        }
        let (blocks, want) = if fixed {
            (count, block)
        } else {
            (1, count as usize)
        };
        let mut given = 0usize;
        for done in 0..blocks {
            let left = count - done;
            let room = want.min(bounce.len());
            let found = medium
                .read(&mut bounce.bytes()[..room])
                .map_err(|e| io_failure(&e, false))?;
            match found {
                TapeRead::Filemark => {
                    let sense = FILEMARK.with(Sense::FILEMARK, left);
                    return Err(Failure::Partial(sense, given));
                }
                TapeRead::EndOfData => {
                    let sense = END_OF_DATA.with(0, left);
                    return Err(Failure::Partial(sense, given));
                }
                TapeRead::Block { len, copied } => {
                    let n = copied.min(want);
                    data_in
                        .write_all(&bounce.bytes()[..n])
                        .map_err(|_| Failure::Overrun)?;
                    given += n;
                    if len != want {
                        // The incorrect length: what was not done of
                        // the count, negative for a longer block.
                        let info = if fixed {
                            left
                        } else {
                            // Two's complement in the field's 32 bits.
                            let as_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
                            as_u32(want).wrapping_sub(as_u32(len))
                        };
                        if fixed || len > want || !sili {
                            let sense = LENGTH.with(Sense::ILI, info);
                            return Err(Failure::Partial(sense, given));
                        }
                    }
                }
            }
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(given)
    }

    /// WRITE (6): `count` bytes as one block, or `count` blocks of the
    /// fixed length.
    fn write(
        &self,
        s: &mut State,
        fixed: bool,
        count: u32,
        data_out: &mut dyn Read,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        if fixed != (s.block != 0) {
            return Err(Sense::INVALID_FIELD.into());
        }
        let block = s.block as usize;
        let medium = Self::medium(s)?;
        if count == 0 {
            return Ok(0);
        }
        let (blocks, len) = if fixed {
            (count, block)
        } else {
            (1, count as usize)
        };
        if len > MAX_BLOCK || len > bounce.len() {
            return Err(Sense::INVALID_FIELD.into());
        }
        for done in 0..blocks {
            let data = &mut bounce.bytes()[..len];
            data_out.read_exact(data).map_err(|_| Failure::Overrun)?;
            match medium.write(data) {
                Ok(false) => {}
                Ok(true) => {
                    // Past the early warning: written, and said.
                    self.writes.fetch_add(1, Ordering::Relaxed);
                    let left = if fixed { count - done - 1 } else { 0 };
                    return Err(EARLY_WARNING.with(Sense::EOM, left).into());
                }
                Err(e) => {
                    let left = if fixed { count - done } else { count };
                    let sense = io_failure(&e, true);
                    return Err(match e {
                        TapeError::Full => OVERFLOW.with(Sense::EOM, left).into(),
                        _ => sense.into(),
                    });
                }
            }
        }
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(0)
    }

    #[allow(clippy::too_many_lines)] // one command a branch
    fn carry_out(
        &self,
        s: &mut State,
        cdb: &[u8],
        luns: usize,
        data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let opcode = cdb[0];
        let passes = matches!(opcode, 0x03 | 0x12 | 0xA0);
        if !passes && let Some(attention) = s.attention.take() {
            return Err(attention.into());
        }
        match opcode {
            // TEST UNIT READY
            0x00 => Self::medium(s).map(|_| 0).map_err(Into::into),
            // REWIND
            0x01 => {
                Self::medium(s)?.rewind();
                Ok(0)
            }
            // REQUEST SENSE
            0x03 => {
                let sense = s
                    .last
                    .take()
                    .or_else(|| s.attention.take())
                    .unwrap_or(Sense::new(0, 0, 0));
                Ok(give(data_in, &sense.bytes(), usize::from(cdb[4])))
            }
            // READ BLOCK LIMITS
            0x05 => Ok(give(data_in, &Self::read_block_limits(), 6)),
            // READ (6)
            0x08 => {
                let (fixed, sili) = (cdb[1] & 0x01 != 0, cdb[1] & 0x02 != 0);
                self.read(s, fixed, sili, be24(cdb, 2), data_in, bounce)
            }
            // WRITE (6)
            0x0A => self.write(s, cdb[1] & 0x01 != 0, be24(cdb, 2), data_out, bounce),
            // WRITE FILEMARKS (6): a count of 0 is a flush.
            0x10 => {
                let count = be24(cdb, 2);
                let medium = Self::medium(s)?;
                let warned = medium.write_filemarks(count).map_err(|e| match e {
                    TapeError::Full => OVERFLOW.with(Sense::EOM, count),
                    other => io_failure(&other, true),
                })?;
                medium.sync().map_err(|_| WRITE_ERROR)?;
                self.flushes.fetch_add(1, Ordering::Relaxed);
                if warned && count > 0 {
                    return Err(EARLY_WARNING.with(Sense::EOM, 0).into());
                }
                Ok(0)
            }
            // SPACE (6)
            0x11 => {
                let count = signed24(cdb, 2);
                let medium = Self::medium(s)?;
                let done = match cdb[1] & 0x0F {
                    0 => medium.space_blocks(count),
                    1 => medium.space_filemarks(count),
                    3 => {
                        medium.space_to_end();
                        Ok(())
                    }
                    _ => return Err(Sense::INVALID_FIELD.into()),
                };
                done.map(|()| 0).map_err(|stop| stopped(stop).into())
            }
            // INQUIRY
            0x12 => {
                let room = usize::from(be16(cdb, 3));
                if cdb[1] & 0x01 != 0 {
                    let page = identity_page(cdb[2], &self.serial).ok_or(Sense::INVALID_FIELD)?;
                    Ok(give(data_in, &page, room))
                } else if cdb[2] != 0 {
                    Err(Sense::INVALID_FIELD.into())
                } else {
                    Ok(give(data_in, &inquiry(0x01, true, PRODUCT), room))
                }
            }
            // MODE SELECT (6) and (10)
            0x15 => Self::mode_select(s, false, usize::from(cdb[4]), data_out),
            0x55 => Self::mode_select(s, true, usize::from(be16(cdb, 7)), data_out),
            // ERASE (6): from here to the end.
            0x19 => {
                Self::medium(s)?.erase().map_err(|e| io_failure(&e, true))?;
                Ok(0)
            }
            // MODE SENSE (6) and (10)
            0x1A => {
                let data = Self::mode_sense(s, false, cdb[1] & 0x08 != 0, cdb[2] & 0x3F)?;
                Ok(give(data_in, &data, usize::from(cdb[4])))
            }
            0x5A => {
                let data = Self::mode_sense(s, true, cdb[1] & 0x08 != 0, cdb[2] & 0x3F)?;
                Ok(give(data_in, &data, usize::from(be16(cdb, 7))))
            }
            // LOAD UNLOAD
            0x1B => {
                let load = cdb[4] & 0x01 != 0;
                if load {
                    Self::medium(s)?.rewind();
                    return Ok(0);
                }
                if s.prevented {
                    return Err(REMOVAL_PREVENTED.into());
                }
                if let Some(m) = s.mounted.take() {
                    let _ = m.medium.sync();
                    s.notices.push(Notice::Unloaded(m.name));
                }
                Ok(0)
            }
            // PREVENT ALLOW MEDIUM REMOVAL
            0x1E => {
                s.prevented = cdb[4] & 0x01 != 0;
                Ok(0)
            }
            // LOCATE (10)
            0x2B => {
                let medium = Self::medium(s)?;
                medium
                    .locate(u64::from(be32(cdb, 3)))
                    .map(|()| 0)
                    .map_err(|stop| stopped(stop).into())
            }
            // READ POSITION
            0x34 => {
                let data = Self::read_position(s, cdb[1] & 0x1F)?;
                Ok(give(data_in, &data, 20))
            }
            // REPORT DENSITY SUPPORT
            0x44 => Ok(give(
                data_in,
                &Self::density_support(s),
                usize::from(be16(cdb, 7)),
            )),
            // REPORT LUNS
            0xA0 => Ok(give(
                data_in,
                &report_luns(luns),
                usize::try_from(be32(cdb, 6)).unwrap_or(0),
            )),
            _ => Err(Sense::INVALID_OPCODE.into()),
        }
    }
}

/// Bytes of the command a group's opcode has.
fn length(opcode: u8) -> usize {
    match opcode >> 5 {
        0 => 6,
        1 | 2 => 10,
        4 => 16,
        5 => 12,
        _ => 0,
    }
}

impl Unit for Tape {
    fn run(
        &self,
        cdb: &[u8],
        luns: usize,
        data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let n = cdb.first().map_or(0, |op| length(*op));
        let result = if n == 0 || cdb.len() < n {
            Err(Failure::Check(Sense::INVALID_OPCODE))
        } else {
            let mut s = self.lock();
            let r = self.carry_out(&mut s, &cdb[..n], luns, data_out, data_in, bounce);
            if let Err(Failure::Check(sense) | Failure::Partial(sense, _)) = r {
                s.last = Some(sense);
            }
            r
        };
        // A filemark or the early warning is a condition, not a refusal.
        let refused = match &result {
            Err(Failure::Check(sense) | Failure::Partial(sense, _)) => sense.key != NO_SENSE,
            Err(Failure::Overrun) => true,
            Ok(_) => false,
        };
        if refused {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn counts(&self) -> Counts {
        Counts {
            reads: self.reads.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            flushes: self.flushes.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
        }
    }

    fn blocks(&self) -> u64 {
        self.lock()
            .mounted
            .as_ref()
            .map_or(0, |m| m.medium.objects())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex as StdMutex};

    #[derive(Clone, Default)]
    struct Mem(Arc<StdMutex<Vec<u8>>>);

    impl Store for Mem {
        fn size(&self) -> io::Result<u64> {
            Ok(self.0.lock().unwrap().len() as u64)
        }
        fn read_at(&self, buf: &mut [u8], at: u64) -> io::Result<()> {
            let v = self.0.lock().unwrap();
            let at = usize::try_from(at).unwrap();
            buf.copy_from_slice(
                v.get(at..at + buf.len())
                    .ok_or_else(|| io::Error::other("end"))?,
            );
            Ok(())
        }
        fn write_at(&self, buf: &[u8], at: u64) -> io::Result<()> {
            let mut v = self.0.lock().unwrap();
            let at = usize::try_from(at).unwrap();
            if v.len() < at + buf.len() {
                v.resize(at + buf.len(), 0);
            }
            v[at..at + buf.len()].copy_from_slice(buf);
            Ok(())
        }
        fn truncate(&self, len: u64) -> io::Result<()> {
            self.0
                .lock()
                .unwrap()
                .resize(usize::try_from(len).unwrap(), 0);
            Ok(())
        }
        fn sync(&self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Run a command: its data, or the sense and the data given.
    fn run(drive: &Tape, cdb: &[u8], out: &[u8]) -> Result<Vec<u8>, (Sense, Vec<u8>)> {
        let mut data = Vec::new();
        let mut bounce = Bounce::new(1 << 20);
        let mut out = out;
        let r = drive.run(cdb, 1, &mut out, &mut data, &mut bounce);
        match r {
            Ok(_) => Ok(data),
            Err(Failure::Check(sense)) => Err((sense, Vec::new())),
            Err(Failure::Partial(sense, n)) => {
                data.truncate(n);
                Err((sense, data))
            }
            Err(Failure::Overrun) => panic!("overrun"),
        }
    }

    fn ready(capacity: u64, protected: bool) -> (Tape, Mem) {
        let t = Tape::new("PROD-TAP").unwrap();
        let mem = Mem::default();
        t.mount(Box::new(mem.clone()), "VOL001", capacity, protected)
            .unwrap();
        // The power-on attention, then the mount's.
        assert!(run(&t, &[0; 6], &[]).is_err());
        (t, mem)
    }

    fn write(t: &Tape, data: &[u8]) -> Result<Vec<u8>, (Sense, Vec<u8>)> {
        let n = u32::try_from(data.len()).unwrap().to_be_bytes();
        run(t, &[0x0A, 0, n[1], n[2], n[3], 0], data)
    }

    fn read(t: &Tape, len: u32, sili: bool) -> Result<Vec<u8>, (Sense, Vec<u8>)> {
        let n = len.to_be_bytes();
        run(
            t,
            &[0x08, if sili { 2 } else { 0 }, n[1], n[2], n[3], 0],
            &[],
        )
    }

    fn filemarks(t: &Tape, n: u8) -> Result<Vec<u8>, (Sense, Vec<u8>)> {
        run(t, &[0x10, 0, 0, 0, n, 0], &[])
    }

    #[test]
    fn a_drive_identifies_itself_and_its_limits() {
        let t = Tape::new("TAP").unwrap();
        let inq = run(&t, &[0x12, 0, 0, 0, 36, 0], &[]).unwrap();
        assert_eq!((inq[0], inq[1]), (0x01, 0x80));
        assert_eq!(&inq[16..32], PRODUCT);
        // The first command after power on is told so; then no volume.
        assert_eq!(run(&t, &[0; 6], &[]).unwrap_err().0, POWER_ON);
        assert_eq!(run(&t, &[0; 6], &[]).unwrap_err().0, NO_MEDIUM);
        let limits = run(&t, &[0x05, 0, 0, 0, 0, 0], &[]).unwrap();
        assert_eq!(limits, [0, 0x10, 0, 0, 0, 1]);
        // Variable blocks to start with: the descriptor's length is 0.
        let mode = run(&t, &[0x1A, 0, 0, 0, 12, 0], &[]).unwrap();
        assert_eq!(mode.len(), 12);
        assert_eq!(mode[3], 8);
        assert_eq!(&mode[9..12], &[0, 0, 0]);
    }

    #[test]
    fn variable_blocks_round_trip_with_filemarks_and_the_end_of_data() {
        let (t, _) = ready(1 << 30, false);
        assert!(run(&t, &[0; 6], &[]).is_ok());
        let label = vec![0xE5; 80];
        let data = vec![0x42; 65536];
        write(&t, &label).unwrap();
        filemarks(&t, 1).unwrap();
        write(&t, &data).unwrap();
        filemarks(&t, 2).unwrap();
        run(&t, &[0x01, 0, 0, 0, 0, 0], &[]).unwrap();
        // The label, read into a larger room without SILI: the length.
        let (sense, got) = read(&t, 256 * 1024, false).unwrap_err();
        assert_eq!(got, label);
        assert_eq!(sense.marks, Sense::ILI);
        assert_eq!(sense.info, Some(256 * 1024 - 80));
        // A filemark: nothing given, the residue the count.
        let (sense, got) = read(&t, 65536, true).unwrap_err();
        assert!(got.is_empty());
        assert_eq!(
            (sense.key, sense.marks, sense.asc, sense.ascq),
            (0, 0x80, 0, 1)
        );
        assert_eq!(sense.info, Some(65536));
        // The save block with SILI into a larger room: no condition.
        assert_eq!(read(&t, 256 * 1024, true).unwrap(), data);
        // A shorter room than the block: ILI, negative residue.
        run(&t, &[0x11, 0, 0xFF, 0xFF, 0xFF, 0], &[]).unwrap(); // back one block
        let (sense, got) = read(&t, 100, true).unwrap_err();
        assert_eq!(got.len(), 100);
        assert_eq!(sense.marks, Sense::ILI);
        assert_eq!(sense.info, Some(100u32.wrapping_sub(65536)));
        // Two filemarks, then the end of data: BLANK CHECK.
        assert!(read(&t, 10, true).is_err());
        assert!(read(&t, 10, true).is_err());
        let (sense, _) = read(&t, 10, true).unwrap_err();
        assert_eq!((sense.key, sense.asc, sense.ascq), (BLANK_CHECK, 0, 5));
        // SPACE to the end of data and READ POSITION.
        run(&t, &[0x11, 3, 0, 0, 0, 0], &[]).unwrap();
        let pos = run(&t, &[0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[]).unwrap();
        assert_eq!(be32(&pos, 4), 5);
        // LOCATE back to the data block and read it.
        run(&t, &[0x2B, 0, 0, 0, 0, 0, 2, 0, 0, 0], &[]).unwrap();
        assert_eq!(read(&t, 65536, false).unwrap(), data);
    }

    #[test]
    fn spacing_reports_where_it_stopped() {
        let (t, _) = ready(1 << 30, false);
        let _ = run(&t, &[0; 6], &[]);
        for _ in 0..3 {
            write(&t, b"block").unwrap();
        }
        filemarks(&t, 1).unwrap();
        run(&t, &[0x01, 0, 0, 0, 0, 0], &[]).unwrap();
        // Five blocks forward: the filemark after three.
        let (sense, _) = run(&t, &[0x11, 0, 0, 0, 5, 0], &[]).unwrap_err();
        assert_eq!((sense.marks, sense.info), (Sense::FILEMARK, Some(2)));
        // Two filemarks forward: the end of data with one left.
        run(&t, &[0x01, 0, 0, 0, 0, 0], &[]).unwrap();
        let (sense, _) = run(&t, &[0x11, 1, 0, 0, 2, 0], &[]).unwrap_err();
        assert_eq!((sense.key, sense.info), (BLANK_CHECK, Some(1)));
        // Ten blocks back from the end: the filemark first.
        let (sense, _) = run(&t, &[0x11, 0, 0xFF, 0xFF, 0xF6, 0], &[]).unwrap_err();
        assert_eq!(sense.marks, Sense::FILEMARK);
        // On back to the beginning: EOM with what was left.
        let (sense, _) = run(&t, &[0x11, 0, 0xFF, 0xFF, 0xF6, 0], &[]).unwrap_err();
        assert_eq!(
            (sense.marks, sense.ascq, sense.info),
            (Sense::EOM, 4, Some(7))
        );
        assert_eq!(
            run(&t, &[0x11, 7, 0, 0, 1, 0], &[]).unwrap_err().0,
            Sense::INVALID_FIELD
        );
    }

    #[test]
    fn the_early_warning_and_the_end_are_reported() {
        let (t, _) = ready(4 << 20, false);
        let _ = run(&t, &[0; 6], &[]);
        let block = vec![1u8; 512 * 1024];
        let mut warned = false;
        let mut written = 0;
        loop {
            match write(&t, &block) {
                Ok(_) => written += 1,
                Err((s, _)) if s.key == NO_SENSE && s.marks == Sense::EOM => {
                    written += 1;
                    warned = true;
                }
                Err((s, _)) => {
                    assert_eq!((s.key, s.marks), (VOLUME_OVERFLOW, Sense::EOM));
                    assert_eq!(s.info, Some(512 * 1024));
                    break;
                }
            }
        }
        assert!(warned);
        assert_eq!(written, 7);
        let pos = run(&t, &[0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[]).unwrap();
        assert_eq!(pos[0] & 0x40, 0x40, "past the early warning");
    }

    #[test]
    fn a_protected_volume_refuses_writes_and_says_so() {
        let (t, _) = ready(1 << 20, true);
        let _ = run(&t, &[0; 6], &[]);
        assert_eq!(write(&t, b"x").unwrap_err().0, WRITE_PROTECTED);
        assert_eq!(filemarks(&t, 1).unwrap_err().0, WRITE_PROTECTED);
        assert_eq!(
            run(&t, &[0x19, 1, 0, 0, 0, 0], &[]).unwrap_err().0,
            WRITE_PROTECTED
        );
        let mode = run(&t, &[0x1A, 0, 0, 0, 12, 0], &[]).unwrap();
        assert_eq!(mode[2] & 0x80, 0x80);
    }

    #[test]
    fn fixed_blocks_are_set_by_mode_select() {
        let (t, _) = ready(1 << 30, false);
        let _ = run(&t, &[0; 6], &[]);
        // Fixed 512-byte blocks.
        let select = [0, 0, 0x10, 8, 0, 0, 0, 0, 0, 0, 2, 0];
        run(&t, &[0x15, 0x10, 0, 0, 12, 0], &select).unwrap();
        let three = vec![9u8; 1536];
        run(&t, &[0x0A, 1, 0, 0, 3, 0], &three).unwrap();
        // Variable writes are refused now.
        assert_eq!(write(&t, b"x").unwrap_err().0, Sense::INVALID_FIELD);
        run(&t, &[0x01, 0, 0, 0, 0, 0], &[]).unwrap();
        assert_eq!(run(&t, &[0x08, 1, 0, 0, 3, 0], &[]).unwrap(), three);
        // Reading on into the end of data: the blocks not read.
        let (sense, _) = run(&t, &[0x08, 1, 0, 0, 2, 0], &[]).unwrap_err();
        assert_eq!((sense.key, sense.info), (BLANK_CHECK, Some(2)));
        // A block length beyond the limit is refused.
        let huge = [0, 0, 0x10, 8, 0, 0, 0, 0, 0, 0x20, 0, 0];
        assert_eq!(
            run(&t, &[0x15, 0x10, 0, 0, 12, 0], &huge).unwrap_err().0,
            Sense::INVALID_PARAMETER
        );
    }

    #[test]
    fn mounting_and_unloading_follow_the_guest_s_prevention() {
        let (t, mem) = ready(1 << 30, false);
        let _ = run(&t, &[0; 6], &[]);
        write(&t, b"kept").unwrap();
        run(&t, &[0x1E, 0, 0, 0, 1, 0], &[]).unwrap();
        assert_eq!(t.unmount(false), Err(UnmountError::Prevented));
        assert_eq!(
            run(&t, &[0x1B, 0, 0, 0, 0, 0], &[]).unwrap_err().0,
            REMOVAL_PREVENTED
        );
        run(&t, &[0x1E, 0, 0, 0, 0, 0], &[]).unwrap();
        // The guest unloads: the host is told, and the data is kept.
        run(&t, &[0x1B, 0, 0, 0, 0, 0], &[]).unwrap();
        assert_eq!(t.notices(), [Notice::Unloaded("VOL001".into())]);
        assert_eq!(t.mounted(), None);
        assert_eq!(run(&t, &[0; 6], &[]).unwrap_err().0, NO_MEDIUM);
        assert_eq!(mem.0.lock().unwrap().len(), 4 + 6);
        // Mounted again: the attention, then the block is there.
        t.mount(Box::new(mem), "VOL001", 1 << 30, false).unwrap();
        assert_eq!(run(&t, &[0; 6], &[]).unwrap_err().0, MEDIUM_CHANGED);
        assert_eq!(read(&t, 4, false).unwrap(), b"kept");
        assert_eq!(t.unmount(true).as_deref(), Ok("VOL001"));
        assert_eq!(t.unmount(true), Err(UnmountError::Empty));
    }

    #[test]
    fn malformed_commands_are_refused() {
        let (t, _) = ready(1 << 20, false);
        let _ = run(&t, &[0; 6], &[]);
        assert_eq!(
            run(&t, &[0xFF; 16], &[]).unwrap_err().0,
            Sense::INVALID_OPCODE
        );
        assert_eq!(run(&t, &[0x08], &[]).unwrap_err().0, Sense::INVALID_OPCODE);
        // A fixed read in variable mode, a read position form not had.
        assert_eq!(
            run(&t, &[0x08, 1, 0, 0, 1, 0], &[]).unwrap_err().0,
            Sense::INVALID_FIELD
        );
        assert_eq!(
            run(&t, &[0x34, 6, 0, 0, 0, 0, 0, 0, 0, 0], &[])
                .unwrap_err()
                .0,
            Sense::INVALID_FIELD
        );
        // A write longer than the data sent: an overrun, nothing written.
        let mut data = Vec::new();
        let mut bounce = Bounce::new(1 << 20);
        let r = t.run(
            &[0x0A, 0, 0, 1, 0, 0],
            1,
            &mut &b"short"[..],
            &mut data,
            &mut bounce,
        );
        assert_eq!(r, Err(Failure::Overrun));
        // A write too large for a block.
        assert_eq!(
            run(&t, &[0x0A, 0, 0x7F, 0xFF, 0xFF, 0], &[]).unwrap_err().0,
            Sense::INVALID_FIELD
        );
    }
}
