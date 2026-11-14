// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A pool disk: a GPT with two partitions, the record partition and
//! the member partition. The record partition holds, from its start:
//!
//! | At | What |
//! |---|---|
//! | 0 | the member label, copy A; the set's write-intent map at 32 KiB |
//! | 64 KiB | the member label, copy B; a member's own map at 32 KiB |
//! | 1 MiB | the resume map, copy A |
//! | 4.5 MiB | the resume map, copy B |
//!
//! The member partition is the set's member, whole.

use abrec::{SLOT_BYTES, Spec};
use diskimg::layout::{Content, Layout, PartDesc, Size};
use uuid::Uuid;

use crate::names::PoolDiskNames;

const MIB: u64 = 1 << 20;

/// Bytes of the record partition.
pub const RECORD_BYTES: u64 = 8 * MIB;
/// The least a member partition may be.
pub const MEMBER_MIN_BYTES: u64 = 256 * MIB;
/// Where the member label's copies start.
pub const LABEL_OFFSET: u64 = 0;
/// Where a write-intent map starts within a copy of the label.
pub const MAP_IN_SLOT: u64 = 32 * 1024;
/// Bytes kept for one write-intent map.
pub const MAP_BYTES: usize = 32 * 1024;
/// Where the set's write-intent map starts.
pub const SET_MAP_OFFSET: u64 = LABEL_OFFSET + MAP_IN_SLOT;
/// Where the map of a member out of service starts.
pub const AWAY_MAP_OFFSET: u64 = LABEL_OFFSET + SLOT_BYTES as u64 + MAP_IN_SLOT;
/// Where the resume map's copies start.
pub const RESUME_OFFSET: u64 = MIB;
/// Bytes of one copy of the resume map: room for 27 TiB.
pub const RESUME_SLOT_BYTES: usize = 3 * (1 << 20) + (1 << 19);
/// A block of a set.
pub const BLOCK: u64 = 4096;

/// GPT type of the record partition.
pub fn record_type(names: &PoolDiskNames) -> Uuid {
    Uuid::from_u128(names.record_type)
}

/// GPT type of the member partition.
pub fn member_type(names: &PoolDiskNames) -> Uuid {
    Uuid::from_u128(names.member_type)
}

/// The resume map as `abrec` sees it.
pub fn resume_spec(names: &PoolDiskNames) -> Spec {
    Spec {
        magic: names.resume_magic,
        version: 1,
        type_guid: record_type(names),
        slot_bytes: RESUME_SLOT_BYTES,
        min_payload: 56,
    }
}

/// The partitions in disk order.
pub fn parts(names: &PoolDiskNames) -> Vec<PartDesc> {
    vec![
        PartDesc {
            name: names.record_name.into(),
            type_guid: record_type(names),
            size: Size::Fixed(RECORD_BYTES),
            content: Content::Wiped,
        },
        PartDesc {
            name: names.member_name.into(),
            type_guid: member_type(names),
            size: Size::Rest {
                min: MEMBER_MIN_BYTES,
            },
            content: Content::Wiped,
        },
    ]
}

/// The layout of a pool disk of `bytes`.
pub fn layout(names: &PoolDiskNames, bytes: u64) -> Result<Layout, String> {
    Layout::for_bytes(&parts(names), bytes).map_err(|e| e.to_string())
}

/// The smallest disk that can be in a pool.
pub fn min_disk_bytes(names: &PoolDiskNames) -> u64 {
    diskimg::layout::min_bytes(&parts(names))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &PoolDiskNames = &PoolDiskNames::DEMO;

    #[test]
    fn the_records_fit_the_record_partition_side_by_side() {
        let label = crate::label::spec(NAMES);
        assert!(MAP_IN_SLOT + MAP_BYTES as u64 <= label.slot_bytes as u64);
        assert_eq!(AWAY_MAP_OFFSET, SET_MAP_OFFSET + label.slot_bytes as u64);
        assert!(LABEL_OFFSET + 2 * label.slot_bytes as u64 <= RESUME_OFFSET);
        assert_eq!(RESUME_OFFSET + 2 * RESUME_SLOT_BYTES as u64, RECORD_BYTES);
        // A resume map has a bit a MiB: 3.5 MiB of them is 27 TiB
        // and more.
        assert!((RESUME_SLOT_BYTES as u64 - 1024) * 8 >= 27 << 20);
    }

    #[test]
    fn a_disk_holds_the_record_and_the_member() {
        let bytes = min_disk_bytes(NAMES) + (1 << 30);
        let l = layout(NAMES, bytes).unwrap();
        assert_eq!(l.parts.len(), 2);
        assert_eq!(l.parts[0].part.bytes(), RECORD_BYTES);
        assert!(l.parts[1].part.bytes() >= MEMBER_MIN_BYTES + (1 << 30) - MIB);
        assert!(layout(NAMES, min_disk_bytes(NAMES) - 4096).is_err());
        assert_ne!(record_type(NAMES), member_type(NAMES));
    }
}
