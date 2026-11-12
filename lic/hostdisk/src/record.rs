// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's A/B record: QS/400's neutral `abrec` under QS/VS's own magic
//! and partition type. The payload, version 2, is sixteen bytes:
//!
//! | Bytes | What |
//! |---|---|
//! | 0 | the default slot: 0 is A, 1 is B |
//! | 1 | the pending slot: 0 none, 1 A, 2 B |
//! | 2 | IPL attempts left for the pending slot |
//! | 3 | reserved, zero |
//! | 4..8 | the release last confirmed in slot A: 1 and V, R, M, or zeros |
//! | 8..12 | the same for slot B |
//! | 12..16 | reserved, zero |
//!
//! The slot an IPL comes from is chosen by firmware boot entry; the
//! record says which slot is the default, which is on trial and what
//! was last confirmed in each.

use std::io::Cursor;

use abrec::{AbError, AbPart, Device, SLOT_BYTES, Slot, Spec};
use vrm::Release;

use crate::layout::RECORD_TYPE_GUID;
use crate::multi::Copies;

/// The record's magic: `QSVSREC`, padded with a zero to the field.
pub const MAGIC: [u8; 8] = *b"QSVSREC\0";

/// The payload's format version. A copy of version 1 (Phase 0's four
/// bytes, which nothing ever read) is not a valid copy.
pub const VERSION: u32 = 2;

/// Bytes of a version 2 payload.
pub const PAYLOAD_LEN: usize = 16;

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
    /// The slot on trial: booted once through `BootNext`, the default
    /// only once it confirms.
    pub pending: Option<Slot>,
    /// IPL attempts left for the pending slot.
    pub attempts: u8,
    /// The release last confirmed in slot A and in slot B.
    pub confirmed: [Option<Release>; 2],
}

/// What an IPL is, by the record and the slot it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipl {
    /// Nothing is pending.
    Normal,
    /// This is the pending slot: it confirms when the base is ready.
    Trial,
    /// A slot was pending and this is not it: it did not confirm.
    FellBack {
        /// The slot that was pending.
        pending: Slot,
    },
}

/// A slot's place in [`HostRecord::confirmed`].
pub fn index(slot: Slot) -> usize {
    match slot {
        Slot::A => 0,
        Slot::B => 1,
    }
}

/// The other slot.
pub fn other(slot: Slot) -> Slot {
    match slot {
        Slot::A => Slot::B,
        Slot::B => Slot::A,
    }
}

/// `A` or `B`.
pub fn letter(slot: Slot) -> char {
    match slot {
        Slot::A => 'A',
        Slot::B => 'B',
    }
}

fn release_bytes(release: Option<Release>) -> [u8; 4] {
    release.map_or([0; 4], |r| [1, r.v, r.r, r.m])
}

/// The release in four payload bytes; an error when they are not one.
fn release_of(bytes: &[u8]) -> Result<Option<Release>, ()> {
    match bytes {
        [0, 0, 0, 0] => Ok(None),
        [1, v, r, m] => Ok(Some(Release {
            v: *v,
            r: *r,
            m: *m,
        })),
        _ => Err(()),
    }
}

impl HostRecord {
    /// A newly built or installed disk: slot A, nothing pending,
    /// nothing confirmed yet.
    pub const INITIAL: HostRecord = HostRecord::new(Slot::A);

    /// A record whose default is `slot`, with nothing else in it.
    pub const fn new(slot: Slot) -> Self {
        Self {
            default_slot: slot,
            pending: None,
            attempts: 0,
            confirmed: [None, None],
        }
    }

    /// The payload bytes.
    pub fn encode(self) -> [u8; PAYLOAD_LEN] {
        let mut out = [0u8; PAYLOAD_LEN];
        out[0] = u8::try_from(index(self.default_slot)).unwrap_or(0);
        out[1] = self.pending.map_or(0, |s| match s {
            Slot::A => 1,
            Slot::B => 2,
        });
        out[2] = self.attempts;
        out[4..8].copy_from_slice(&release_bytes(self.confirmed[0]));
        out[8..12].copy_from_slice(&release_bytes(self.confirmed[1]));
        out
    }

    /// From payload bytes; `None` when they are not a version 2 payload.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let p = payload.get(..PAYLOAD_LEN)?;
        let default_slot = match p[0] {
            0 => Slot::A,
            1 => Slot::B,
            _ => return None,
        };
        let pending = match p[1] {
            0 => None,
            1 => Some(Slot::A),
            2 => Some(Slot::B),
            _ => return None,
        };
        if p[3] != 0 || p[12..] != [0; 4] {
            return None;
        }
        Some(Self {
            default_slot,
            pending,
            attempts: p[2],
            confirmed: [release_of(&p[4..8]).ok()?, release_of(&p[8..12]).ok()?],
        })
    }

    /// What an IPL from `booted` is, and the record as it stands once
    /// the IPL has begun: a trial has used one attempt; a slot that
    /// did not confirm is no longer pending.
    pub fn at_ipl(self, booted: Slot) -> (Ipl, HostRecord) {
        match self.pending {
            None => (Ipl::Normal, self),
            Some(pending) if pending == booted => (
                Ipl::Trial,
                HostRecord {
                    attempts: self.attempts.saturating_sub(1),
                    ..self
                },
            ),
            Some(pending) => (
                Ipl::FellBack { pending },
                HostRecord {
                    pending: None,
                    attempts: 0,
                    ..self
                },
            ),
        }
    }

    /// The record once `booted`, running `release`, has confirmed: it
    /// is the default and nothing is pending.
    #[must_use]
    pub fn confirm(self, booted: Slot, release: Release) -> HostRecord {
        let mut confirmed = self.confirmed;
        confirmed[index(booted)] = Some(release);
        HostRecord {
            default_slot: booted,
            pending: None,
            attempts: 0,
            confirmed,
        }
    }

    /// The record with `release` noted as the one last confirmed in
    /// `booted`, and nothing else changed: an IPL that was no trial.
    #[must_use]
    pub fn with_release(self, booted: Slot, release: Release) -> HostRecord {
        let mut confirmed = self.confirmed;
        confirmed[index(booted)] = Some(release);
        HostRecord { confirmed, ..self }
    }

    /// `default A, pending B (1 attempt left)`, for the console.
    pub fn summary(&self) -> String {
        let pending = self.pending.map_or_else(
            || "nothing pending".to_string(),
            |s| {
                let plural = if self.attempts == 1 { "" } else { "s" };
                format!(
                    "pending {} ({} attempt{plural} left)",
                    letter(s),
                    self.attempts
                )
            },
        );
        format!("default {}, {pending}", letter(self.default_slot))
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
    read(&mut AbPart::open(SPEC, Cursor::new(bytes.to_vec())))
}

/// The newest valid record on `part` and its sequence.
pub fn read<D: Device>(part: &mut AbPart<D>) -> Result<(HostRecord, u64), AbError> {
    let stored = part.read()?;
    let record = HostRecord::decode(&stored.payload)
        .ok_or(AbError::Malformed("not a version 2 host record"))?;
    Ok((record, stored.sequence))
}

/// Write `record` to the copy `abrec` chooses: the older one, so the
/// newest valid copy survives a write that is cut short.
pub fn write<D: Device>(part: &mut AbPart<D>, record: HostRecord) -> Result<u64, AbError> {
    part.write(&record.encode()).map(|(_, sequence)| sequence)
}

/// The newest valid record on any disk of `copies` and its sequence.
pub fn read_all<D: Device>(copies: &mut Copies<D>) -> Result<(HostRecord, u64), AbError> {
    let stored = copies.read()?;
    let record = HostRecord::decode(&stored.payload)
        .ok_or(AbError::Malformed("not a version 2 host record"))?;
    Ok((record, stored.sequence))
}

/// Write `record` to every disk of `copies`: the sequence, and how
/// many disks took it.
pub fn write_all<D: Device>(
    copies: &mut Copies<D>,
    record: HostRecord,
) -> Result<(u64, usize), AbError> {
    copies.write(&record.encode())
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1R0M0: Release = Release { v: 1, r: 0, m: 0 };

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
        let full = HostRecord {
            default_slot: Slot::A,
            pending: Some(Slot::B),
            attempts: 1,
            confirmed: [Some(V1R0M0), Some(Release { v: 1, r: 2, m: 3 })],
        };
        for r in [HostRecord::INITIAL, HostRecord::new(Slot::B), full] {
            assert_eq!(HostRecord::decode(&r.encode()), Some(r));
        }
        assert_eq!(
            full.encode(),
            [0, 2, 1, 0, 1, 1, 0, 0, 1, 1, 2, 3, 0, 0, 0, 0]
        );
        let good = full.encode();
        for (at, value) in [(0, 2), (1, 3), (3, 1), (4, 2), (8, 9), (15, 1)] {
            let mut bad = good;
            bad[at] = value;
            assert_eq!(HostRecord::decode(&bad), None, "byte {at}");
        }
        // Phase 0's version 1 payload is too short to be one.
        assert_eq!(HostRecord::decode(&[0, 0, 0, 0]), None);
    }

    #[test]
    fn an_empty_partition_has_no_record() {
        let zeros = vec![0u8; 2 * SPEC.slot_bytes];
        assert!(matches!(read_bytes(&zeros), Err(AbError::NoValidCopy)));
    }

    #[test]
    fn a_pending_slot_is_tried_once_and_confirms() {
        let pending = HostRecord {
            pending: Some(Slot::B),
            attempts: 1,
            ..HostRecord::INITIAL
        };
        let (ipl, at_ipl) = pending.at_ipl(Slot::B);
        assert_eq!(ipl, Ipl::Trial);
        assert_eq!((at_ipl.pending, at_ipl.attempts), (Some(Slot::B), 0));
        assert_eq!(at_ipl.default_slot, Slot::A);
        let confirmed = at_ipl.confirm(Slot::B, V1R0M0);
        assert_eq!(
            confirmed,
            HostRecord {
                default_slot: Slot::B,
                pending: None,
                attempts: 0,
                confirmed: [None, Some(V1R0M0)],
            }
        );
        assert_eq!(confirmed.at_ipl(Slot::B), (Ipl::Normal, confirmed));
    }

    #[test]
    fn a_pending_slot_that_did_not_confirm_is_cleared_on_the_next_ipl() {
        // The trial used its attempt and never confirmed; the firmware
        // then booted the default.
        let tried = HostRecord {
            pending: Some(Slot::B),
            attempts: 0,
            confirmed: [Some(V1R0M0), None],
            ..HostRecord::INITIAL
        };
        let (ipl, after) = tried.at_ipl(Slot::A);
        assert_eq!(ipl, Ipl::FellBack { pending: Slot::B });
        assert_eq!(
            after,
            HostRecord {
                default_slot: Slot::A,
                pending: None,
                attempts: 0,
                confirmed: [Some(V1R0M0), None],
            }
        );
    }

    #[test]
    fn an_ipl_that_is_no_trial_only_notes_its_release() {
        // Booted from B by hand while A is the default: A stays it.
        let noted = HostRecord::INITIAL.with_release(Slot::B, V1R0M0);
        assert_eq!(noted.default_slot, Slot::A);
        assert_eq!(noted.confirmed, [None, Some(V1R0M0)]);
        assert_eq!(noted.with_release(Slot::B, V1R0M0), noted);
    }

    #[test]
    fn writes_alternate_between_the_copies() {
        let mut part = AbPart::open(
            SPEC,
            Cursor::new(initial_bytes(HostRecord::INITIAL).unwrap()),
        );
        let next = HostRecord::INITIAL.confirm(Slot::A, V1R0M0);
        assert_eq!(write(&mut part, next).unwrap(), 2);
        assert_eq!(read(&mut part).unwrap(), (next, 2));
        let bytes = part.into_device().into_inner();
        // Copy A still holds the first record; copy B the second.
        assert_eq!(&bytes[SPEC.slot_bytes..SPEC.slot_bytes + 8], b"QSVSREC\0");
        assert_eq!(read_bytes(&bytes).unwrap().1, 2);
    }

    #[test]
    fn the_summary_is_what_the_console_shows() {
        assert_eq!(HostRecord::INITIAL.summary(), "default A, nothing pending");
        let pending = HostRecord {
            pending: Some(Slot::B),
            attempts: 1,
            ..HostRecord::INITIAL
        };
        assert_eq!(pending.summary(), "default A, pending B (1 attempt left)");
    }
}
