// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The SCSI commands a disk LUN takes, decoded from the command
//! descriptor block, and the sense data for everything it does not.
//! Decoding reads the block's bytes and nothing else: no length it
//! names is trusted here.

/// Sense data: why a command ended with a check condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sense {
    /// The sense key.
    pub key: u8,
    /// The additional sense code.
    pub asc: u8,
    /// Its qualifier.
    pub ascq: u8,
}

impl Sense {
    const ILLEGAL_REQUEST: u8 = 0x05;
    const MEDIUM_ERROR: u8 = 0x03;

    /// No such command.
    pub const INVALID_OPCODE: Sense = Sense::illegal(0x20, 0x00);
    /// A field of the command is not one this LUN takes.
    pub const INVALID_FIELD: Sense = Sense::illegal(0x24, 0x00);
    /// The blocks named are not all on the LUN.
    pub const LBA_OUT_OF_RANGE: Sense = Sense::illegal(0x21, 0x00);
    /// The parameter list's length is wrong.
    pub const PARAMETER_LENGTH: Sense = Sense::illegal(0x1A, 0x00);
    /// A field of the parameter list is wrong.
    pub const INVALID_PARAMETER: Sense = Sense::illegal(0x26, 0x00);
    /// The backing store failed a read.
    pub const READ_ERROR: Sense = Sense::medium(0x11, 0x00);
    /// The backing store failed a write.
    pub const WRITE_ERROR: Sense = Sense::medium(0x0C, 0x00);

    const fn illegal(asc: u8, ascq: u8) -> Sense {
        Sense {
            key: Self::ILLEGAL_REQUEST,
            asc,
            ascq,
        }
    }

    const fn medium(asc: u8, ascq: u8) -> Sense {
        Sense {
            key: Self::MEDIUM_ERROR,
            asc,
            ascq,
        }
    }

    /// Fixed-format sense data.
    pub fn bytes(&self) -> [u8; 18] {
        let mut b = [0u8; 18];
        b[0] = 0x70;
        b[2] = self.key;
        b[7] = 10;
        b[12] = self.asc;
        b[13] = self.ascq;
        b
    }
}

/// A command a disk LUN takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// TEST UNIT READY, START STOP UNIT, PREVENT ALLOW MEDIUM REMOVAL:
    /// nothing to do.
    Nothing,
    /// REQUEST SENSE.
    RequestSense {
        /// Bytes the initiator has room for.
        room: usize,
    },
    /// INQUIRY.
    Inquiry {
        /// A vital product data page, or the standard data.
        page: Option<u8>,
        /// Bytes the initiator has room for.
        room: usize,
    },
    /// REPORT LUNS.
    ReportLuns {
        /// Bytes the initiator has room for.
        room: usize,
    },
    /// READ CAPACITY (10).
    ReadCapacity10,
    /// READ CAPACITY (16).
    ReadCapacity16 {
        /// Bytes the initiator has room for.
        room: usize,
    },
    /// READ (6), (10), (12) or (16).
    Read {
        /// The first block.
        lba: u64,
        /// How many.
        blocks: u32,
    },
    /// WRITE (6), (10), (12) or (16).
    Write {
        /// The first block.
        lba: u64,
        /// How many.
        blocks: u32,
        /// Force unit access: on stable storage before the command ends.
        fua: bool,
    },
    /// VERIFY (10) or (16) without a comparison: the blocks exist.
    Verify {
        /// The first block.
        lba: u64,
        /// How many.
        blocks: u32,
    },
    /// SYNCHRONIZE CACHE (10) or (16): everything, whatever range it names.
    SynchronizeCache,
    /// MODE SENSE (6) or (10).
    ModeSense {
        /// The ten-byte form.
        ten: bool,
        /// Leave the block descriptor out.
        no_block_descriptor: bool,
        /// The changeable values are asked for.
        changeable: bool,
        /// The page.
        page: u8,
        /// Bytes the initiator has room for.
        room: usize,
    },
    /// UNMAP.
    Unmap {
        /// Bytes of the parameter list.
        length: usize,
    },
    /// WRITE SAME (10) or (16).
    WriteSame {
        /// The first block.
        lba: u64,
        /// How many.
        blocks: u32,
        /// Unmap the blocks if that reads back the same.
        unmap: bool,
        /// No data-out block: the pattern is zeros.
        no_data: bool,
    },
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn be64(b: &[u8], at: usize) -> u64 {
    (u64::from(be32(b, at)) << 32) | u64::from(be32(b, at + 4))
}

/// Bytes of a command by its group: the top three bits of the opcode.
fn length(opcode: u8) -> Option<usize> {
    match opcode >> 5 {
        0 => Some(6),
        1 | 2 => Some(10),
        4 => Some(16),
        5 => Some(12),
        _ => None,
    }
}

/// Decode the command in `cdb`.
pub fn decode(cdb: &[u8]) -> Result<Command, Sense> {
    let opcode = *cdb.first().ok_or(Sense::INVALID_OPCODE)?;
    let needed = length(opcode).ok_or(Sense::INVALID_OPCODE)?;
    let c = cdb.get(..needed).ok_or(Sense::INVALID_FIELD)?;
    Ok(match opcode {
        0x00 | 0x1B | 0x1E => Command::Nothing,
        0x03 => Command::RequestSense {
            room: usize::from(c[4]),
        },
        0x12 => {
            let vital = c[1] & 1 != 0;
            if !vital && c[2] != 0 {
                return Err(Sense::INVALID_FIELD);
            }
            Command::Inquiry {
                page: vital.then_some(c[2]),
                room: usize::from(be16(c, 3)),
            }
        }
        0xA0 => Command::ReportLuns {
            room: be32(c, 6) as usize,
        },
        0x25 => Command::ReadCapacity10,
        0x9E => {
            if c[1] & 0x1F != 0x10 {
                return Err(Sense::INVALID_FIELD);
            }
            Command::ReadCapacity16 {
                room: be32(c, 10) as usize,
            }
        }
        0x08 | 0x0A => {
            let lba = u64::from(be32(c, 0) & 0x001F_FFFF);
            let blocks = if c[4] == 0 { 256 } else { u32::from(c[4]) };
            if opcode == 0x08 {
                Command::Read { lba, blocks }
            } else {
                Command::Write {
                    lba,
                    blocks,
                    fua: false,
                }
            }
        }
        0x28 | 0x2A | 0x2F => {
            let lba = u64::from(be32(c, 2));
            let blocks = u32::from(be16(c, 7));
            transfer(opcode & 0x0F, c[1], lba, blocks)?
        }
        0xA8 | 0xAA => transfer(opcode & 0x0F, c[1], u64::from(be32(c, 2)), be32(c, 6))?,
        0x88 | 0x8A | 0x8F => transfer(opcode & 0x0F, c[1], be64(c, 2), be32(c, 10))?,
        0x35 | 0x91 => Command::SynchronizeCache,
        0x1A => Command::ModeSense {
            ten: false,
            no_block_descriptor: c[1] & 0x08 != 0,
            changeable: c[2] >> 6 == 1,
            page: c[2] & 0x3F,
            room: usize::from(c[4]),
        },
        0x5A => Command::ModeSense {
            ten: true,
            no_block_descriptor: c[1] & 0x08 != 0,
            changeable: c[2] >> 6 == 1,
            page: c[2] & 0x3F,
            room: usize::from(be16(c, 7)),
        },
        0x42 => Command::Unmap {
            length: usize::from(be16(c, 7)),
        },
        0x41 => Command::WriteSame {
            lba: u64::from(be32(c, 2)),
            blocks: u32::from(be16(c, 7)),
            unmap: c[1] & 0x08 != 0,
            no_data: false,
        },
        0x93 => Command::WriteSame {
            lba: be64(c, 2),
            blocks: be32(c, 10),
            unmap: c[1] & 0x08 != 0,
            no_data: c[1] & 0x01 != 0,
        },
        _ => return Err(Sense::INVALID_OPCODE),
    })
}

/// A read, write or verify by the low nibble of its opcode.
fn transfer(kind: u8, flags: u8, lba: u64, blocks: u32) -> Result<Command, Sense> {
    Ok(match kind {
        0x08 => Command::Read { lba, blocks },
        0x0A => Command::Write {
            lba,
            blocks,
            fua: flags & 0x08 != 0,
        },
        _ => {
            // BYTCHK: a comparison with data-out is not offered.
            if flags & 0x06 != 0 {
                return Err(Sense::INVALID_FIELD);
            }
            Command::Verify { lba, blocks }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_of_every_size_decode() {
        let read10 = [0x28, 0, 0x12, 0x34, 0x56, 0x78, 0, 0x01, 0x02, 0];
        assert_eq!(
            decode(&read10),
            Ok(Command::Read {
                lba: 0x1234_5678,
                blocks: 0x102
            })
        );
        let mut write16 = [0u8; 16];
        write16[0] = 0x8A;
        write16[1] = 0x08;
        write16[2..10].copy_from_slice(&0x1_0000_0000u64.to_be_bytes());
        write16[10..14].copy_from_slice(&8u32.to_be_bytes());
        assert_eq!(
            decode(&write16),
            Ok(Command::Write {
                lba: 0x1_0000_0000,
                blocks: 8,
                fua: true
            })
        );
        assert_eq!(
            decode(&[0x08, 0x1F, 0xFF, 0xFF, 0, 0]),
            Ok(Command::Read {
                lba: 0x1F_FFFF,
                blocks: 256
            })
        );
        let read12 = [0xA8, 0, 0, 0, 0, 9, 0, 0, 1, 0, 0, 0];
        assert_eq!(
            decode(&read12),
            Ok(Command::Read {
                lba: 9,
                blocks: 256
            })
        );
        // A CDB of 32 bytes, as virtio-scsi carries it, decodes by its group.
        let mut padded = [0u8; 32];
        padded[..10].copy_from_slice(&read10);
        assert_eq!(decode(&padded), decode(&read10));
    }

    #[test]
    fn the_enquiries_decode() {
        assert_eq!(
            decode(&[0x12, 1, 0x83, 0x01, 0x00, 0]),
            Ok(Command::Inquiry {
                page: Some(0x83),
                room: 256
            })
        );
        assert_eq!(
            decode(&[0x12, 0, 0x83, 0, 36, 0]),
            Err(Sense::INVALID_FIELD)
        );
        let mut capacity16 = [0u8; 16];
        capacity16[0] = 0x9E;
        capacity16[1] = 0x10;
        capacity16[13] = 32;
        assert_eq!(
            decode(&capacity16),
            Ok(Command::ReadCapacity16 { room: 32 })
        );
        capacity16[1] = 0x11;
        assert_eq!(decode(&capacity16), Err(Sense::INVALID_FIELD));
        assert_eq!(
            decode(&[0x1A, 0x08, 0x48, 0, 0xFF, 0]),
            Ok(Command::ModeSense {
                ten: false,
                no_block_descriptor: true,
                changeable: true,
                page: 8,
                room: 255
            })
        );
        assert_eq!(
            decode(&[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Ok(Command::SynchronizeCache)
        );
        assert_eq!(decode(&[0x00, 0, 0, 0, 0, 0]), Ok(Command::Nothing));
    }

    #[test]
    fn what_a_disk_does_not_take_is_refused_with_sense() {
        assert_eq!(decode(&[]), Err(Sense::INVALID_OPCODE));
        assert_eq!(decode(&[0xFF; 16]), Err(Sense::INVALID_OPCODE));
        assert_eq!(decode(&[0x15, 0, 0, 0, 0, 0]), Err(Sense::INVALID_OPCODE));
        // A command cut short.
        assert_eq!(decode(&[0x28, 0, 0]), Err(Sense::INVALID_FIELD));
        // VERIFY with a comparison.
        assert_eq!(
            decode(&[0x2F, 0x02, 0, 0, 0, 0, 0, 0, 1, 0]),
            Err(Sense::INVALID_FIELD)
        );
        // Every opcode decodes or is refused; none panics.
        for opcode in 0..=255u8 {
            for fill in [0x00, 0xFF] {
                let mut cdb = [fill; 32];
                cdb[0] = opcode;
                let _ = decode(&cdb);
                let _ = decode(&cdb[..7]);
            }
        }
        let sense = Sense::LBA_OUT_OF_RANGE.bytes();
        assert_eq!((sense[0], sense[2], sense[12]), (0x70, 5, 0x21));
    }
}
