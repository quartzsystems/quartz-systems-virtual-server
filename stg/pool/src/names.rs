// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a pool disk carries that is the host's own. QS/VS's values are
//! `release::POOL_DISK`; a test there holds each one.

/// The types, names and magics of a pool disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PoolDiskNames {
    /// GPT type of the record partition, as the GUID's 128 bits.
    pub record_type: u128,
    /// GPT type of the member partition.
    pub member_type: u128,
    /// GPT name of the record partition.
    pub record_name: &'static str,
    /// GPT name of the member partition.
    pub member_name: &'static str,
    /// The magic of the member label.
    pub label_magic: [u8; 8],
    /// The magic of the resume map's record.
    pub resume_magic: [u8; 8],
    /// The magic the protection engine encodes its resume map under.
    pub resume_map_magic: [u8; 8],
    /// The magic of the write-intent maps.
    pub intent_magic: [u8; 8],
    /// The magic of a parity set's partial-parity log slots.
    pub ppl_magic: [u8; 8],
    /// The magic of a parity set's state page.
    pub parity_state_magic: [u8; 8],
}

impl PoolDiskNames {
    /// Values for tests, none of them any product's.
    pub const DEMO: PoolDiskNames = PoolDiskNames {
        record_type: 0x0d5e_c0de_0000_4000_8000_0000_0000_0011,
        member_type: 0x0d5e_c0de_0000_4000_8000_0000_0000_0012,
        record_name: "QSDMPOOL-R",
        member_name: "QSDMPOOL",
        label_magic: *b"QSDMPOOL",
        resume_magic: *b"QSDMPRSM",
        resume_map_magic: *b"QSDMPRMP",
        intent_magic: *b"QSDMPIMP",
        ppl_magic: *b"QSDMPPPL",
        parity_state_magic: *b"QSDMPSTA",
    };
}
