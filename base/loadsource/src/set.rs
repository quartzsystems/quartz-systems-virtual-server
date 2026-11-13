// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The label of a mirrored load source. QS/400's `protect` keeps
//! nothing on a member that says what it is, so the host records the
//! set itself, beside the A/B record on every load-source disk and
//! outside the set: which data partitions are the members, each one's
//! state, the set's size, and whether the set was stopped in order.
//! It is read before `/data` exists, so it cannot be a state file.
//!
//! Each disk holds its own copy. They agree while the disks run
//! together. When each disk was started without the other, each copy
//! says its own member is in step and the other is not: the units ran
//! [`apart`], and nothing on the disks can say which holds what the
//! operator wants.
//!
//! The payload's first byte is its version. Version 2 added the time a
//! copy was written; a copy of version 1 reads with that time unknown.
//!
//! The record partition holds, from its start: the A/B record (two
//! copies), the label (two copies), and the map of a resynchronisation
//! under way (two copies).

use abrec::{SLOT_BYTES, Spec};
use appliance::LoadSourceNames;
use uuid::Uuid;

use crate::layout::record_type;

/// Where the label's copies start in the record partition.
pub const LABEL_OFFSET: u64 = 2 * SLOT_BYTES as u64;
/// Where the resume map's copies start.
pub const RESUME_OFFSET: u64 = 4 * SLOT_BYTES as u64;
/// Bytes of one copy of the resume map: the two fill the megabyte.
pub const RESUME_SLOT_BYTES: usize = 384 * 1024;

/// The label as `abrec` sees it.
pub fn label_spec(names: &LoadSourceNames) -> Spec {
    Spec {
        magic: names.label_magic,
        version: 1,
        type_guid: record_type(names),
        slot_bytes: SLOT_BYTES,
        min_payload: HEAD_LEN,
    }
}

/// The resume map as `abrec` sees it; the payload is `protect`'s,
/// encoded under the appliance's `resume_map_magic`.
pub fn resume_spec(names: &LoadSourceNames) -> Spec {
    Spec {
        magic: names.resume_magic,
        version: 1,
        type_guid: record_type(names),
        slot_bytes: RESUME_SLOT_BYTES,
        min_payload: 56,
    }
}

/// Bytes a block of the set has.
pub const BLOCK: u64 = 4096;

/// Bytes before the members in a version 1 payload.
const HEAD_LEN: usize = 28;
/// The same in version 2: the time written follows.
const HEAD_LEN_2: usize = 36;
const MEMBER_LEN: usize = 84;
const TEXT_LEN: usize = 32;

/// A member's state, with `protect`'s unit table bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberState {
    /// In step with the other member.
    Active,
    /// Not written to; its data is stale.
    Suspended,
    /// Being brought back in step.
    Resuming,
    /// Present but failing I/O.
    Failed,
    /// Not found.
    Missing,
}

impl MemberState {
    /// The byte on disk.
    pub fn as_u8(self) -> u8 {
        match self {
            MemberState::Active => 1,
            MemberState::Suspended => 2,
            MemberState::Resuming => 3,
            MemberState::Failed => 4,
            MemberState::Missing => 5,
        }
    }

    /// From the byte on disk.
    pub fn from_u8(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => MemberState::Active,
            2 => MemberState::Suspended,
            3 => MemberState::Resuming,
            4 => MemberState::Failed,
            5 => MemberState::Missing,
            _ => return None,
        })
    }

    /// As the disk unit displays word it.
    pub fn word(self) -> &'static str {
        match self {
            MemberState::Active => "Active",
            MemberState::Suspended => "Suspended",
            MemberState::Resuming => "Resuming",
            MemberState::Failed => "Failed",
            MemberState::Missing => "Missing",
        }
    }
}

/// One member of the set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The GPT GUID of its data partition: how the member is found.
    pub partition: Uuid,
    /// Its state when the label was written.
    pub state: MemberState,
    /// The serial number of its disk, for a display when it is gone.
    pub serial: String,
    /// The location code its disk had, for the same.
    pub location: String,
}

/// The label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetLabel {
    /// The set's identity.
    pub id: Uuid,
    /// The set's size in 4096-byte blocks.
    pub blocks: u64,
    /// Whether the set was stopped in order. A set found open was cut
    /// off while it ran: its members may differ and are resynchronised.
    pub clean: bool,
    /// When this copy was written, in seconds since the epoch; 0 when
    /// that is not known.
    pub written: u64,
    /// The members, in order.
    pub members: Vec<Member>,
}

impl SetLabel {
    /// The payload bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; HEAD_LEN_2 + self.members.len() * MEMBER_LEN];
        out[0] = 2;
        out[1] = u8::from(self.clean);
        out[2] = u8::try_from(self.members.len()).unwrap_or(u8::MAX);
        out[4..20].copy_from_slice(self.id.as_bytes());
        out[20..28].copy_from_slice(&self.blocks.to_le_bytes());
        out[28..36].copy_from_slice(&self.written.to_le_bytes());
        for (i, m) in self.members.iter().enumerate() {
            let at = HEAD_LEN_2 + i * MEMBER_LEN;
            out[at..at + 16].copy_from_slice(m.partition.as_bytes());
            out[at + 16] = m.state.as_u8();
            for (text, at) in [(&m.serial, at + 20), (&m.location, at + 20 + TEXT_LEN)] {
                let n = text.len().min(TEXT_LEN);
                out[at..at + n].copy_from_slice(&text.as_bytes()[..n]);
            }
        }
        out
    }

    /// From payload bytes.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        if payload.len() < HEAD_LEN || payload[1] > 1 {
            return None;
        }
        let head = match payload[0] {
            1 => HEAD_LEN,
            2 => HEAD_LEN_2,
            _ => return None,
        };
        let count = usize::from(payload[2]);
        if count < 2 || payload.len() < head + count * MEMBER_LEN {
            return None;
        }
        let written = match head {
            HEAD_LEN_2 => u64::from_le_bytes(payload[28..36].try_into().ok()?),
            _ => 0,
        };
        let mut members = Vec::with_capacity(count);
        for i in 0..count {
            let m = &payload[head + i * MEMBER_LEN..][..MEMBER_LEN];
            let text = |at: usize| {
                let bytes: Vec<u8> = m[at..at + TEXT_LEN]
                    .iter()
                    .copied()
                    .take_while(|b| *b != 0)
                    .collect();
                String::from_utf8(bytes).ok()
            };
            members.push(Member {
                partition: Uuid::from_slice(&m[..16]).ok()?,
                state: MemberState::from_u8(m[16])?,
                serial: text(20)?,
                location: text(20 + TEXT_LEN)?,
            });
        }
        Some(Self {
            id: Uuid::from_slice(&payload[4..20]).ok()?,
            blocks: u64::from_le_bytes(payload[20..28].try_into().ok()?),
            clean: payload[1] == 1,
            written,
            members,
        })
    }

    /// The state this copy gives the member on `partition`; `None`
    /// when it names no such member.
    pub fn state_of(&self, partition: Uuid) -> Option<MemberState> {
        self.members
            .iter()
            .find(|m| m.partition == partition)
            .map(|m| m.state)
    }

    /// The label once the operator has said that this copy's disk is
    /// the one to keep: a member this copy has out of step, whose disk
    /// is there (`present`), is stale and is written again from the
    /// kept one. Written at `now`.
    #[must_use]
    pub fn keeping(&self, present: impl Fn(Uuid) -> bool, now: u64) -> SetLabel {
        let mut kept = self.clone();
        kept.written = now;
        for m in &mut kept.members {
            if m.state != MemberState::Active && present(m.partition) {
                m.state = MemberState::Suspended;
            }
        }
        kept
    }

    /// Whether every member is in step.
    pub fn protected(&self) -> bool {
        self.members.iter().all(|m| m.state == MemberState::Active)
    }
}

/// The disks whose copies of the label say they ran apart. `own` is
/// each disk that is there: the GUID of its data partition and the
/// newest label on that disk itself. A disk is listed when its copy
/// has its own member in step and another disk's not, while that other
/// disk's copy says the reverse: each was started without the other
/// and went on alone.
///
/// The rule is confirmed (`docs/OPEN_QUESTIONS.md` item 20).
/// The host does not choose between such disks by the sequence of
/// their records: it stops before the data file system is mounted and
/// asks the operator which to keep.
pub fn apart(own: &[(Uuid, SetLabel)]) -> Vec<usize> {
    let alone = |(mine, label): &(Uuid, SetLabel), other: Uuid| {
        label.state_of(*mine) == Some(MemberState::Active)
            && label.state_of(other) != Some(MemberState::Active)
    };
    (0..own.len())
        .filter(|i| {
            own.iter().enumerate().any(|(j, other)| {
                j != *i
                    && own[*i].1.id == other.1.id
                    && alone(&own[*i], other.0)
                    && alone(other, own[*i].0)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label() -> SetLabel {
        SetLabel {
            id: Uuid::from_u128(7),
            blocks: 190_000,
            clean: true,
            written: 1_794_478_923,
            members: vec![
                Member {
                    partition: Uuid::from_u128(1),
                    state: MemberState::Active,
                    serial: "DEMOHD0".into(),
                    location: "P0000-B00-D05-F0-U0".into(),
                },
                Member {
                    partition: Uuid::from_u128(2),
                    state: MemberState::Resuming,
                    serial: String::new(),
                    location: String::new(),
                },
            ],
        }
    }

    #[test]
    fn the_label_round_trips() {
        let l = label();
        assert_eq!(SetLabel::decode(&l.encode()), Some(l.clone()));
        assert!(!l.protected());
        let mut open = l;
        open.clean = false;
        open.members[1].state = MemberState::Active;
        assert_eq!(SetLabel::decode(&open.encode()), Some(open.clone()));
        assert!(open.protected());
    }

    /// A label as version 1 had it: no time written.
    fn version_1(label: &SetLabel) -> Vec<u8> {
        let mut bytes = label.encode();
        bytes.drain(HEAD_LEN..HEAD_LEN_2);
        bytes[0] = 1;
        bytes
    }

    #[test]
    fn a_version_1_label_reads_with_its_time_unknown() {
        let l = label();
        assert_eq!(l.encode()[0], 2);
        let old = SetLabel::decode(&version_1(&l)).unwrap();
        assert_eq!(old.written, 0);
        assert_eq!(SetLabel { written: 0, ..l }, old);
    }

    /// The pair as each disk records it after running without the other.
    fn alone(mine: u128, written: u64) -> SetLabel {
        let mut l = label();
        l.written = written;
        for m in &mut l.members {
            m.state = if m.partition == Uuid::from_u128(mine) {
                MemberState::Active
            } else {
                MemberState::Missing
            };
        }
        l
    }

    #[test]
    fn units_that_each_ran_without_the_other_are_apart() {
        let (one, two) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut both = label();
        both.members[1].state = MemberState::Active;
        // In step; one behind and being written; one that ran alone
        // while the other did not run at all.
        assert!(apart(&[(one, both.clone()), (two, both.clone())]).is_empty());
        assert!(apart(&[(one, label()), (two, label())]).is_empty());
        assert!(apart(&[(one, alone(1, 50)), (two, both.clone())]).is_empty());
        assert!(apart(&[(one, alone(1, 50))]).is_empty());
        // Each ran without the other.
        assert_eq!(apart(&[(one, alone(1, 50)), (two, alone(2, 60))]), [0, 1]);
        // Not the same set: not this rule's.
        let mut foreign = alone(2, 60);
        foreign.id = Uuid::from_u128(8);
        assert!(apart(&[(one, alone(1, 50)), (two, foreign)]).is_empty());
        // A third disk that took unit 2's place while unit 1 ran alone
        // is in step with unit 1 and apart from the old unit 2.
        let three = Uuid::from_u128(3);
        let mut replaced = both;
        replaced.members[1].partition = three;
        assert_eq!(
            apart(&[
                (one, replaced.clone()),
                (two, alone(2, 60)),
                (three, replaced)
            ]),
            [0, 1, 2]
        );
    }

    #[test]
    fn the_kept_unit_s_label_has_the_other_stale() {
        let two = Uuid::from_u128(2);
        let kept = alone(1, 50).keeping(|_| true, 70);
        assert_eq!(kept.written, 70);
        assert_eq!(kept.state_of(Uuid::from_u128(1)), Some(MemberState::Active));
        assert_eq!(kept.state_of(two), Some(MemberState::Suspended));
        assert_eq!(kept.state_of(Uuid::from_u128(3)), None);
        // Both disks hold it now: they are apart no more.
        assert!(apart(&[(Uuid::from_u128(1), kept.clone()), (two, kept)]).is_empty());
        // A member whose disk is not there stays missing.
        let gone = alone(1, 50).keeping(|_| false, 70);
        assert_eq!(gone.state_of(two), Some(MemberState::Missing));
    }

    #[test]
    fn what_is_not_a_label_is_refused() {
        let bytes = label().encode();
        assert_eq!(SetLabel::decode(&bytes[..40]), None);
        let mut version = bytes.clone();
        version[0] = 3;
        assert_eq!(SetLabel::decode(&version), None);
        let mut state = bytes.clone();
        state[HEAD_LEN_2 + 16] = 9;
        assert_eq!(SetLabel::decode(&state), None);
        let mut one = bytes;
        one[2] = 1;
        assert_eq!(SetLabel::decode(&one), None);
    }

    #[test]
    fn the_three_records_fit_the_partition_side_by_side() {
        let names = &LoadSourceNames::DEMO;
        assert_eq!(
            LABEL_OFFSET,
            2 * crate::record::spec(names).slot_bytes as u64
        );
        assert_eq!(
            RESUME_OFFSET,
            LABEL_OFFSET + 2 * label_spec(names).slot_bytes as u64
        );
        assert_eq!(resume_spec(names).slot_bytes, RESUME_SLOT_BYTES);
        assert_eq!(
            RESUME_OFFSET + 2 * RESUME_SLOT_BYTES as u64,
            crate::layout::RECORD_BYTES
        );
        for state in [1, 2, 3, 4, 5] {
            assert_eq!(MemberState::from_u8(state).unwrap().as_u8(), state);
        }
    }
}
