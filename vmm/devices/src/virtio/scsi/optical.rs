// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A virtual optical drive: a read-only CD/DVD device of 2048-byte
//! blocks, whose medium is an image from the media library. It answers
//! what Linux's `sr` and `cdrom` drivers send, which is all QS/400 sends
//! to it (`docs/QS400_GUEST.md`): INQUIRY, TEST UNIT READY, REQUEST
//! SENSE, READ CAPACITY, READ (10) and (12), READ TOC, GET
//! CONFIGURATION, GET EVENT STATUS NOTIFICATION, MODE SENSE for the
//! capabilities page, START STOP UNIT and its eject, PREVENT ALLOW
//! MEDIUM REMOVAL, READ DISC INFORMATION, and REPORT LUNS.
//!
//! The medium is loaded and unloaded by the host while the partition
//! runs: the next command is told by a unit attention and the next
//! event poll by a media event. An eject from the guest unloads it and
//! the host is told; removal the guest has prevented makes the host's
//! unload fail unless it is forced.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::backing::{Backing, Bounce};
use super::cdb::Sense;
use super::lun::{Counts, Failure, MAX_TRANSFER_BLOCKS};
use super::unit::{Unit, give, identity_page, inquiry, report_luns, valid_serial};

/// Bytes of a block.
pub const BLOCK: u32 = 2048;
/// An image larger than a CD holds is presented as a DVD.
const CD_BYTES: u64 = 900 << 20;

const PRODUCT: &[u8; 16] = b"VIRTUAL OPTICAL ";

/// The MMC profiles.
const PROFILE_CD_ROM: u16 = 0x0008;
const PROFILE_DVD_ROM: u16 = 0x0010;

/// A media event for GET EVENT STATUS NOTIFICATION.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MediaEvent {
    None,
    EjectRequest,
    NewMedia,
    MediaRemoval,
}

impl MediaEvent {
    fn code(self) -> u8 {
        match self {
            MediaEvent::None => 0,
            MediaEvent::EjectRequest => 1,
            MediaEvent::NewMedia => 2,
            MediaEvent::MediaRemoval => 3,
        }
    }
}

/// What the drive told the host of its own accord.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The guest ejected the medium, named here: it is unloaded.
    Ejected(String),
}

/// Why the host's unload was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UnloadError {
    /// No medium is loaded.
    #[error("no medium is loaded")]
    Empty,
    /// The guest has prevented the medium's removal.
    #[error("the partition has prevented the medium's removal")]
    Prevented,
}

struct Medium {
    store: Box<dyn Backing>,
    name: String,
    blocks: u64,
}

struct State {
    medium: Option<Medium>,
    prevented: bool,
    /// A unit attention the next command reports.
    attention: Option<Sense>,
    /// The sense of the last command that failed, for REQUEST SENSE.
    last: Option<Sense>,
    event: MediaEvent,
    notices: Vec<Notice>,
}

/// A virtual optical drive.
pub struct Optical {
    serial: String,
    state: Mutex<State>,
    reads: AtomicU64,
    refused: AtomicU64,
}

/// Not ready: no medium.
const NO_MEDIUM: Sense = Sense::new(0x02, 0x3A, 0x00);
/// Unit attention: the medium may have changed.
const MEDIUM_CHANGED: Sense = Sense::new(0x06, 0x28, 0x00);
/// Unit attention: the device was reset (its first command).
const POWER_ON: Sense = Sense::new(0x06, 0x29, 0x00);
/// Data protect: a write to a read-only medium.
const WRITE_PROTECTED: Sense = Sense::new(0x07, 0x27, 0x00);
/// Illegal request: the medium's removal is prevented.
const REMOVAL_PREVENTED: Sense = Sense::new(0x05, 0x53, 0x02);

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Frames of a logical block address: 75 a second, from 2 seconds.
fn msf(lba: u32) -> [u8; 4] {
    let f = lba + 150;
    #[allow(clippy::cast_possible_truncation)] // a minute is under 256
    [0, (f / 4500) as u8, ((f / 75) % 60) as u8, (f % 75) as u8]
}

impl Optical {
    /// An empty drive with the serial number `serial`.
    pub fn new(serial: &str) -> Option<Self> {
        valid_serial(serial).then(|| Self {
            serial: serial.to_string(),
            state: Mutex::new(State {
                medium: None,
                prevented: false,
                attention: Some(POWER_ON),
                last: None,
                event: MediaEvent::None,
                notices: Vec::new(),
            }),
            reads: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Load the image in `store`, named `name`, in place of any medium.
    /// An image that is not whole blocks is refused.
    pub fn load(&self, store: Box<dyn Backing>, name: &str) -> Result<(), &'static str> {
        let bytes = store.bytes();
        if bytes == 0 || !bytes.is_multiple_of(u64::from(BLOCK)) {
            return Err("an image is a whole number of 2048-byte blocks");
        }
        let mut s = self.lock();
        s.medium = Some(Medium {
            store,
            name: name.to_string(),
            blocks: bytes / u64::from(BLOCK),
        });
        s.attention = Some(MEDIUM_CHANGED);
        s.event = MediaEvent::NewMedia;
        Ok(())
    }

    /// Unload the medium; when the guest prevented it, only `forced`.
    /// The name of the medium unloaded.
    pub fn unload(&self, forced: bool) -> Result<String, UnloadError> {
        let mut s = self.lock();
        if s.medium.is_none() {
            return Err(UnloadError::Empty);
        }
        if s.prevented && !forced {
            s.event = MediaEvent::EjectRequest;
            return Err(UnloadError::Prevented);
        }
        let name = s.medium.take().map(|m| m.name).unwrap_or_default();
        s.prevented = false;
        s.attention = Some(MEDIUM_CHANGED);
        s.event = MediaEvent::MediaRemoval;
        Ok(name)
    }

    /// The name of the medium loaded.
    pub fn loaded(&self) -> Option<String> {
        self.lock().medium.as_ref().map(|m| m.name.clone())
    }

    /// Whether the guest has prevented the medium's removal.
    pub fn prevented(&self) -> bool {
        self.lock().prevented
    }

    /// What the drive did of its own accord since the last look.
    pub fn notices(&self) -> Vec<Notice> {
        std::mem::take(&mut self.lock().notices)
    }

    fn capabilities_page() -> [u8; 30] {
        // The capabilities and mechanical status page: reads CD-R/RW
        // and DVD-ROM media, writes none; a tray that locks and ejects.
        let mut p = [0u8; 30];
        p[0] = 0x2A;
        p[1] = 28;
        p[2] = 0x3B; // reads CD-R, CD-RW, method 2, DVD-ROM, DVD-R, DVD-RAM
        p[4] = 0x71; // mode 2 forms, multisession
        p[5] = 0x01; // CD-DA commands
        p[6] = 0x29; // tray, eject, lock
        p[8..10].copy_from_slice(&(176u16 * 8).to_be_bytes()); // maximum speed (obsolete)
        p
    }

    fn mode_sense(ten: bool, page: u8, room: usize) -> Result<Vec<u8>, Sense> {
        if page != 0x2A && page != 0x3F {
            return Err(Sense::INVALID_FIELD);
        }
        let body = Self::capabilities_page();
        let mut out = Vec::with_capacity(body.len() + 8);
        #[allow(clippy::cast_possible_truncation)] // thirty bytes
        if ten {
            out.extend_from_slice(&(body.len() as u16 + 6).to_be_bytes());
            out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        } else {
            out.extend_from_slice(&[body.len() as u8 + 3, 0, 0, 0]);
        }
        out.extend_from_slice(&body);
        out.truncate(room.max(out.len()).min(out.len()));
        Ok(out)
    }

    fn profile(blocks: u64) -> u16 {
        if blocks * u64::from(BLOCK) > CD_BYTES {
            PROFILE_DVD_ROM
        } else {
            PROFILE_CD_ROM
        }
    }

    /// GET CONFIGURATION: the profile list, core, morphing, removable
    /// medium, random readable and the CD or DVD read feature, from
    /// `start` on; `rt` 1 the current ones, 2 the one named.
    fn configuration(medium: Option<u64>, rt: u8, start: u16) -> Vec<u8> {
        let current = medium.map_or(0, Self::profile);
        let read_feature = if current == PROFILE_DVD_ROM {
            0x001F
        } else {
            0x001E
        };
        let feature = |code: u16, current: bool, data: &[u8]| {
            let mut f = code.to_be_bytes().to_vec();
            #[allow(clippy::cast_possible_truncation)] // a few bytes
            f.extend_from_slice(&[0x02 | u8::from(current), data.len() as u8]);
            f.extend_from_slice(data);
            f
        };
        let has = medium.is_some();
        let mut profiles = Vec::new();
        for p in [PROFILE_DVD_ROM, PROFILE_CD_ROM] {
            profiles.extend_from_slice(&p.to_be_bytes());
            profiles.extend_from_slice(&[u8::from(p == current), 0]);
        }
        let features: Vec<(u16, Vec<u8>)> = vec![
            (0x0000, feature(0x0000, true, &profiles)),
            (0x0001, feature(0x0001, true, &[0, 0, 0, 8, 0x01, 0, 0, 0])),
            (0x0002, feature(0x0002, true, &[0x02, 0, 0, 0])),
            (0x0003, feature(0x0003, true, &[0x29, 0, 0, 0])),
            (
                0x0010,
                feature(0x0010, has, &[0, 0, 0x08, 0, 0, 0x01, 0, 0]),
            ),
            (read_feature, feature(read_feature, has, &[0, 0, 0, 0])),
        ];
        let mut out = vec![0u8; 8];
        out[6..8].copy_from_slice(&current.to_be_bytes());
        for (code, bytes) in features {
            let wanted = match rt {
                1 => code >= start && (bytes[2] & 0x01 != 0),
                2 => code == start,
                _ => code >= start,
            };
            if wanted {
                out.extend_from_slice(&bytes);
            }
        }
        let len = u32::try_from(out.len() - 4).unwrap_or(0);
        out[..4].copy_from_slice(&len.to_be_bytes());
        out
    }

    /// READ TOC: format 0 (the tracks, track 1 and the lead-out) or 1
    /// (the first session).
    fn toc(blocks: u64, format: u8, msf_form: bool, track: u8) -> Result<Vec<u8>, Sense> {
        let address = |lba: u32| -> [u8; 4] {
            if msf_form {
                msf(lba)
            } else {
                lba.to_be_bytes()
            }
        };
        let end = u32::try_from(blocks).unwrap_or(u32::MAX);
        let mut out = vec![0u8, 0, 1, 1];
        match format {
            0 => {
                if track > 1 && track != 0xAA {
                    return Err(Sense::INVALID_FIELD);
                }
                if track <= 1 {
                    out.extend_from_slice(&[0, 0x14, 1, 0]);
                    out.extend_from_slice(&address(0));
                }
                out.extend_from_slice(&[0, 0x14, 0xAA, 0]);
                out.extend_from_slice(&address(end));
            }
            1 => {
                out.extend_from_slice(&[0, 0x14, 1, 0]);
                out.extend_from_slice(&address(0));
            }
            _ => return Err(Sense::INVALID_FIELD),
        }
        let len = u16::try_from(out.len() - 2).unwrap_or(0);
        out[..2].copy_from_slice(&len.to_be_bytes());
        Ok(out)
    }

    fn disc_information(blocks: u64) -> Vec<u8> {
        let mut d = vec![0u8; 34];
        d[1] = 32;
        d[2] = 0x0E; // complete disc, complete last session
        d[3] = 1; // first track
        d[4] = 1; // sessions
        d[5] = 1; // first track of the last session
        d[6] = 1; // last track of the last session
        d[7] = 0x20; // unrestricted use
        let end = u32::try_from(blocks).unwrap_or(u32::MAX);
        d[16..20].copy_from_slice(&msf(end));
        d[20..24].copy_from_slice(&msf(end));
        d
    }

    fn read(
        &self,
        s: &State,
        lba: u64,
        blocks: u32,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let medium = s.medium.as_ref().ok_or(NO_MEDIUM)?;
        if blocks > MAX_TRANSFER_BLOCKS {
            return Err(Sense::INVALID_FIELD.into());
        }
        let end = lba
            .checked_add(u64::from(blocks))
            .ok_or(Sense::LBA_OUT_OF_RANGE)?;
        if end > medium.blocks {
            return Err(Sense::LBA_OUT_OF_RANGE.into());
        }
        let mut offset = lba * u64::from(BLOCK);
        let mut left = usize::try_from(u64::from(blocks) * u64::from(BLOCK))
            .map_err(|_| Sense::INVALID_FIELD)?;
        let total = left;
        while left > 0 {
            let n = left.min(bounce.len());
            let chunk = &mut bounce.bytes()[..n];
            medium
                .store
                .read_at(chunk, offset)
                .map_err(|_| Sense::READ_ERROR)?;
            data_in.write_all(chunk).map_err(|_| Failure::Overrun)?;
            offset += n as u64;
            left -= n;
        }
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(total)
    }

    #[allow(clippy::too_many_lines)] // one command a branch
    fn carry_out(
        &self,
        s: &mut State,
        cdb: &[u8],
        luns: usize,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let opcode = cdb[0];
        // These neither report nor clear a unit attention.
        let passes = matches!(opcode, 0x03 | 0x12 | 0x46 | 0x4A | 0xA0);
        if !passes && let Some(attention) = s.attention.take() {
            return Err(attention.into());
        }
        let ready = |s: &State| s.medium.as_ref().map(|m| m.blocks).ok_or(NO_MEDIUM);
        match opcode {
            // TEST UNIT READY
            0x00 => ready(s).map(|_| 0).map_err(Into::into),
            // REQUEST SENSE
            0x03 => {
                let room = usize::from(cdb[4]);
                let sense = s
                    .last
                    .take()
                    .or_else(|| s.attention.take())
                    .unwrap_or(Sense::new(0, 0, 0));
                Ok(give(data_in, &sense.bytes(), room))
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
                    Ok(give(data_in, &inquiry(0x05, true, PRODUCT), room))
                }
            }
            // START STOP UNIT
            0x1B => {
                let (load_eject, start) = (cdb[4] & 0x02 != 0, cdb[4] & 0x01 != 0);
                if load_eject && !start {
                    if s.prevented {
                        return Err(REMOVAL_PREVENTED.into());
                    }
                    if let Some(m) = s.medium.take() {
                        s.event = MediaEvent::MediaRemoval;
                        s.notices.push(Notice::Ejected(m.name));
                    }
                }
                Ok(0)
            }
            // PREVENT ALLOW MEDIUM REMOVAL
            0x1E => {
                s.prevented = cdb[4] & 0x01 != 0;
                Ok(0)
            }
            // READ CAPACITY (10)
            0x25 => {
                let blocks = ready(s)?;
                let last = u32::try_from(blocks - 1).unwrap_or(u32::MAX);
                let mut b = last.to_be_bytes().to_vec();
                b.extend_from_slice(&BLOCK.to_be_bytes());
                Ok(give(data_in, &b, 8))
            }
            // READ (10)
            0x28 => {
                let (lba, n) = (u64::from(be32(cdb, 2)), u32::from(be16(cdb, 7)));
                self.read(s, lba, n, data_in, bounce)
            }
            // READ (12)
            0xA8 => {
                let (lba, n) = (u64::from(be32(cdb, 2)), be32(cdb, 6));
                self.read(s, lba, n, data_in, bounce)
            }
            // WRITE (10), (12), WRITE AND VERIFY: a read-only medium.
            0x2A | 0xAA | 0x2E => {
                ready(s)?;
                Err(WRITE_PROTECTED.into())
            }
            // SEEK (10), VERIFY (10): the blocks are there.
            0x2B | 0x2F => {
                let blocks = ready(s)?;
                if u64::from(be32(cdb, 2)) >= blocks {
                    return Err(Sense::LBA_OUT_OF_RANGE.into());
                }
                Ok(0)
            }
            // READ TOC/PMA/ATIP
            0x43 => {
                let blocks = ready(s)?;
                let room = usize::from(be16(cdb, 7));
                let format = if cdb[2] & 0x0F != 0 {
                    cdb[2] & 0x0F
                } else {
                    cdb[9] >> 6
                };
                let toc = Self::toc(blocks, format, cdb[1] & 0x02 != 0, cdb[6])?;
                Ok(give(data_in, &toc, room))
            }
            // GET CONFIGURATION
            0x46 => {
                let room = usize::from(be16(cdb, 7));
                let medium = s.medium.as_ref().map(|m| m.blocks);
                let data = Self::configuration(medium, cdb[1] & 0x03, be16(cdb, 2));
                Ok(give(data_in, &data, room))
            }
            // GET EVENT STATUS NOTIFICATION, polled only.
            0x4A => {
                if cdb[1] & 0x01 == 0 {
                    return Err(Sense::INVALID_FIELD.into());
                }
                let room = usize::from(be16(cdb, 7));
                if cdb[4] & 0x10 == 0 {
                    // No class asked for that the drive has: no event.
                    return Ok(give(data_in, &[0, 2, 0x80, 0x10], room));
                }
                let event = std::mem::replace(&mut s.event, MediaEvent::None);
                let status = if s.medium.is_some() { 0x02 } else { 0x00 };
                let data = [0, 6, 0x04, 0x10, event.code(), status, 0, 0];
                Ok(give(data_in, &data, room))
            }
            // READ DISC INFORMATION
            0x51 => {
                let blocks = ready(s)?;
                let room = usize::from(be16(cdb, 7));
                Ok(give(data_in, &Self::disc_information(blocks), room))
            }
            // MODE SENSE (6) and (10)
            0x1A => {
                let data = Self::mode_sense(false, cdb[2] & 0x3F, usize::from(cdb[4]))?;
                Ok(give(data_in, &data, usize::from(cdb[4])))
            }
            0x5A => {
                let room = usize::from(be16(cdb, 7));
                let data = Self::mode_sense(true, cdb[2] & 0x3F, room)?;
                Ok(give(data_in, &data, room))
            }
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

impl Unit for Optical {
    fn run(
        &self,
        cdb: &[u8],
        luns: usize,
        _data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let n = cdb.first().map_or(0, |op| length(*op));
        let result = if n == 0 || cdb.len() < n {
            Err(Failure::Check(Sense::INVALID_OPCODE))
        } else {
            let mut s = self.lock();
            let r = self.carry_out(&mut s, &cdb[..n], luns, data_in, bounce);
            if let Err(Failure::Check(sense)) = r {
                s.last = Some(sense);
            }
            r
        };
        if result.is_err() {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn counts(&self) -> Counts {
        Counts {
            reads: self.reads.load(Ordering::Relaxed),
            writes: 0,
            flushes: 0,
            refused: self.refused.load(Ordering::Relaxed),
        }
    }

    fn blocks(&self) -> u64 {
        self.lock().medium.as_ref().map_or(0, |m| m.blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::super::backing::memory::MemoryStore;
    use super::*;
    use std::io;

    fn image(blocks: usize) -> Box<dyn Backing> {
        let store = MemoryStore::new(blocks * 2048, false);
        for b in 0..blocks {
            let mut block = vec![0u8; 2048];
            block[..8].copy_from_slice(&(b as u64).to_be_bytes());
            store.write_at(&block, (b * 2048) as u64).unwrap();
        }
        Box::new(store)
    }

    /// Run a command; its data, or the sense it failed with.
    fn run(drive: &Optical, cdb: &[u8]) -> Result<Vec<u8>, Sense> {
        let mut out = Vec::new();
        let mut bounce = Bounce::new(65536);
        match drive.run(cdb, 1, &mut io::empty(), &mut out, &mut bounce) {
            Ok(_) => Ok(out),
            Err(Failure::Check(sense) | Failure::Partial(sense, _)) => Err(sense),
            Err(other) => panic!("{other:?}"),
        }
    }

    fn cdb10(op: u8) -> [u8; 10] {
        let mut c = [0u8; 10];
        c[0] = op;
        c
    }

    #[test]
    fn an_empty_drive_says_so_and_identifies_as_a_removable_cd() {
        let d = Optical::new("PROD-OPT").unwrap();
        // The first command after power on is told so.
        assert_eq!(run(&d, &[0, 0, 0, 0, 0, 0]), Err(POWER_ON));
        assert_eq!(run(&d, &[0, 0, 0, 0, 0, 0]), Err(NO_MEDIUM));
        let inq = run(&d, &[0x12, 0, 0, 0, 96, 0]).unwrap();
        assert_eq!(inq[0], 0x05);
        assert_eq!(inq[1], 0x80);
        assert_eq!(&inq[16..32], PRODUCT);
        let serial = run(&d, &[0x12, 1, 0x80, 0, 64, 0]).unwrap();
        assert_eq!(&serial[4..], b"PROD-OPT");
        // REQUEST SENSE gives the last failure.
        assert_eq!(run(&d, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Err(NO_MEDIUM));
        let sense = run(&d, &[0x03, 0, 0, 0, 18, 0]).unwrap();
        assert_eq!((sense[2], sense[12]), (0x02, 0x3A));
        assert!(Optical::new("bad serial!").is_none());
    }

    #[test]
    fn a_loaded_image_is_read_and_cannot_be_written() {
        let d = Optical::new("OPT").unwrap();
        d.load(image(40), "QS400").unwrap();
        // The load is told once, then the drive is ready.
        assert_eq!(run(&d, &[0, 0, 0, 0, 0, 0]), Err(MEDIUM_CHANGED));
        assert!(run(&d, &[0, 0, 0, 0, 0, 0]).is_ok());
        let cap = run(&d, &cdb10(0x25)).unwrap();
        assert_eq!(cap, [0, 0, 0, 39, 0, 0, 8, 0]);
        let mut read = cdb10(0x28);
        read[5] = 16;
        read[8] = 2;
        let data = run(&d, &read).unwrap();
        assert_eq!(data.len(), 4096);
        assert_eq!(&data[..8], &16u64.to_be_bytes());
        assert_eq!(&data[2048..2056], &17u64.to_be_bytes());
        read[5] = 39;
        assert_eq!(run(&d, &read), Err(Sense::LBA_OUT_OF_RANGE));
        assert_eq!(run(&d, &cdb10(0x2A)), Err(WRITE_PROTECTED));
        assert_eq!(d.counts().reads, 1);
        assert!(d.load(image(0), "EMPTY").is_err());
    }

    #[test]
    fn the_toc_configuration_and_capabilities_describe_one_data_track() {
        let d = Optical::new("OPT").unwrap();
        d.load(image(1000), "QS400").unwrap();
        let _ = run(&d, &[0, 0, 0, 0, 0, 0]);
        let mut toc = cdb10(0x43);
        toc[8] = 100;
        let t = run(&d, &toc).unwrap();
        assert_eq!(&t[..4], &[0, 18, 1, 1]);
        assert_eq!(t[6], 1);
        assert_eq!(t[14], 0xAA);
        assert_eq!(&t[16..20], &1000u32.to_be_bytes());
        // Format 1: the first session starts at 0.
        toc[2] = 1;
        let session = run(&d, &toc).unwrap();
        assert_eq!(&session[8..12], &[0, 0, 0, 0]);
        let mut conf = cdb10(0x46);
        conf[8] = 255;
        let c = run(&d, &conf).unwrap();
        assert_eq!(be16(&c, 6), PROFILE_CD_ROM);
        let mut sense10 = cdb10(0x5A);
        sense10[2] = 0x2A;
        sense10[8] = 255;
        let m = run(&d, &sense10).unwrap();
        assert_eq!(m[8], 0x2A);
        sense10[2] = 0x08;
        assert_eq!(run(&d, &sense10), Err(Sense::INVALID_FIELD));
    }

    #[test]
    fn media_events_and_ejects_are_told() {
        let d = Optical::new("OPT").unwrap();
        let mut gesn = cdb10(0x4A);
        gesn[1] = 1;
        gesn[4] = 0x10;
        gesn[8] = 8;
        d.load(image(4), "ONE").unwrap();
        let e = run(&d, &gesn).unwrap();
        assert_eq!((e[4], e[5]), (2, 0x02), "new media, present");
        let e = run(&d, &gesn).unwrap();
        assert_eq!(e[4], 0, "told once");
        // Not polled: refused.
        gesn[1] = 0;
        assert_eq!(run(&d, &gesn), Err(Sense::INVALID_FIELD));
        // The guest prevents removal: the host's unload is refused
        // unless forced, and the guest's eject too.
        let _ = run(&d, &[0, 0, 0, 0, 0, 0]);
        run(&d, &[0x1E, 0, 0, 0, 1, 0]).unwrap();
        assert!(d.prevented());
        assert_eq!(d.unload(false), Err(UnloadError::Prevented));
        assert_eq!(run(&d, &[0x1B, 0, 0, 0, 0x02, 0]), Err(REMOVAL_PREVENTED));
        // Allowed again: the guest ejects, and the host is told.
        run(&d, &[0x1E, 0, 0, 0, 0, 0]).unwrap();
        run(&d, &[0x1B, 0, 0, 0, 0x02, 0]).unwrap();
        assert_eq!(d.loaded(), None);
        assert_eq!(d.notices(), [Notice::Ejected("ONE".into())]);
        assert_eq!(d.unload(false), Err(UnloadError::Empty));
        // Forced: unloaded whatever the guest said.
        d.load(image(4), "TWO").unwrap();
        let _ = run(&d, &[0, 0, 0, 0, 0, 0]);
        run(&d, &[0x1E, 0, 0, 0, 1, 0]).unwrap();
        assert_eq!(d.unload(true).as_deref(), Ok("TWO"));
        assert_eq!(run(&d, &[0, 0, 0, 0, 0, 0]), Err(MEDIUM_CHANGED));
        assert_eq!(run(&d, &[0, 0, 0, 0, 0, 0]), Err(NO_MEDIUM));
    }

    #[test]
    fn malformed_commands_are_refused() {
        let d = Optical::new("OPT").unwrap();
        let _ = run(&d, &[0, 0, 0, 0, 0, 0]);
        assert_eq!(run(&d, &[0xFF; 16]), Err(Sense::INVALID_OPCODE));
        assert_eq!(run(&d, &[0x28, 0, 0]), Err(Sense::INVALID_OPCODE));
        assert_eq!(run(&d, &[]), Err(Sense::INVALID_OPCODE));
        d.load(image(4), "X").unwrap();
        let _ = run(&d, &[0, 0, 0, 0, 0, 0]);
        // A count that overflows the address.
        let mut read = [0u8; 12];
        read[0] = 0xA8;
        read[2..6].copy_from_slice(&u32::MAX.to_be_bytes());
        read[6..10].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(run(&d, &read), Err(Sense::INVALID_FIELD));
        let mut toc = cdb10(0x43);
        toc[2] = 5;
        assert_eq!(run(&d, &toc), Err(Sense::INVALID_FIELD));
    }
}
