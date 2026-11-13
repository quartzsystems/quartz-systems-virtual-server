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

const HEAD_LEN: usize = 28;
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
    /// The members, in order.
    pub members: Vec<Member>,
}

impl SetLabel {
    /// The payload bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![0u8; HEAD_LEN + self.members.len() * MEMBER_LEN];
        out[0] = 1;
        out[1] = u8::from(self.clean);
        out[2] = u8::try_from(self.members.len()).unwrap_or(u8::MAX);
        out[4..20].copy_from_slice(self.id.as_bytes());
        out[20..28].copy_from_slice(&self.blocks.to_le_bytes());
        for (i, m) in self.members.iter().enumerate() {
            let at = HEAD_LEN + i * MEMBER_LEN;
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
        if payload.len() < HEAD_LEN || payload[0] != 1 || payload[1] > 1 {
            return None;
        }
        let count = usize::from(payload[2]);
        if count < 2 || payload.len() < HEAD_LEN + count * MEMBER_LEN {
            return None;
        }
        let mut members = Vec::with_capacity(count);
        for i in 0..count {
            let m = &payload[HEAD_LEN + i * MEMBER_LEN..][..MEMBER_LEN];
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
            members,
        })
    }

    /// Whether every member is in step.
    pub fn protected(&self) -> bool {
        self.members.iter().all(|m| m.state == MemberState::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label() -> SetLabel {
        SetLabel {
            id: Uuid::from_u128(7),
            blocks: 190_000,
            clean: true,
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

    #[test]
    fn what_is_not_a_label_is_refused() {
        let bytes = label().encode();
        assert_eq!(SetLabel::decode(&bytes[..40]), None);
        let mut version = bytes.clone();
        version[0] = 2;
        assert_eq!(SetLabel::decode(&version), None);
        let mut state = bytes.clone();
        state[HEAD_LEN + 16] = 9;
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
