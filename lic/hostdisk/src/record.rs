// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's A/B record: QS/400's neutral `abrec` under QS/VS's own magic
//! and partition type. The payload, version 1, is four bytes: the slot
//! the host IPLs from by default, then three reserved zero bytes. Later
//! phases grow the payload under a new version.

use std::io::Cursor;

use abrec::{AbError, AbPart, SLOT_BYTES, Slot, Spec};

use crate::layout::RECORD_TYPE_GUID;

/// The record's magic: `QSVSREC`, padded with a zero to the field.
pub const MAGIC: [u8; 8] = *b"QSVSREC\0";

/// The payload's format version.
pub const VERSION: u32 = 1;

/// Bytes of a version 1 payload.
pub const PAYLOAD_LEN: usize = 4;

/// The record as `abrec` sees it.
pub const SPEC: Spec = Spec {
    magic: MAGIC,
    version: VERSION,
    type_guid: RECORD_TYPE_GUID,
    slot_bytes: SLOT_BYTES,
    min_payload: PAYLOAD_LEN,
};

/// What the record holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostRecord {
    /// The slot the host IPLs from by default.
    pub default_slot: Slot,
}

impl HostRecord {
    /// A newly built or installed disk: slot A.
    pub const INITIAL: HostRecord = HostRecord {
        default_slot: Slot::A,
    };

    /// The payload bytes.
    pub fn encode(self) -> [u8; PAYLOAD_LEN] {
        let slot = match self.default_slot {
            Slot::A => 0,
            Slot::B => 1,
        };
        [slot, 0, 0, 0]
    }

    /// From payload bytes; `None` when they are not a version 1 payload.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let default_slot = match payload.get(..PAYLOAD_LEN)? {
            [0, 0, 0, 0] => Slot::A,
            [1, 0, 0, 0] => Slot::B,
            _ => return None,
        };
        Some(Self { default_slot })
    }
}

/// The first bytes of a new record partition: `record` in copy A with
/// sequence 1, copy B empty. Both copies' room, the rest zero.
pub fn initial_bytes(record: HostRecord) -> Result<Vec<u8>, AbError> {
    let mut part = AbPart::open(SPEC, Cursor::new(vec![0u8; 2 * SPEC.slot_bytes]));
    part.init(&record.encode())?;
    Ok(part.into_device().into_inner())
}

/// The newest valid record in `bytes` (the head of the partition) and
/// its sequence.
pub fn read_bytes(bytes: &[u8]) -> Result<(HostRecord, u64), AbError> {
    let mut part = AbPart::open(SPEC, Cursor::new(bytes.to_vec()));
    let stored = part.read()?;
    let record = HostRecord::decode(&stored.payload)
        .ok_or(AbError::Malformed("not a version 1 host record"))?;
    Ok((record, stored.sequence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_partition_reads_back_as_slot_a_sequence_1() {
        let bytes = initial_bytes(HostRecord::INITIAL).unwrap();
        assert_eq!(bytes.len(), 2 * SPEC.slot_bytes);
        assert_eq!(&bytes[..8], b"QSVSREC\0");
        // Copy B is untouched.
        assert!(bytes[SPEC.slot_bytes..].iter().all(|b| *b == 0));
        assert_eq!(read_bytes(&bytes).unwrap(), (HostRecord::INITIAL, 1));
    }

    #[test]
    fn the_payload_round_trips_and_refuses_junk() {
        for slot in [Slot::A, Slot::B] {
            let r = HostRecord { default_slot: slot };
            assert_eq!(HostRecord::decode(&r.encode()), Some(r));
        }
        assert_eq!(HostRecord::decode(&[2, 0, 0, 0]), None);
        assert_eq!(HostRecord::decode(&[0, 0, 1, 0]), None);
        assert_eq!(HostRecord::decode(&[0]), None);
    }

    #[test]
    fn an_empty_partition_has_no_record() {
        let zeros = vec![0u8; 2 * SPEC.slot_bytes];
        assert!(matches!(read_bytes(&zeros), Err(AbError::NoValidCopy)));
    }
}
