// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The member label: on every disk of a pool, its members and its hot
//! spares alike, in the record partition, two copies through QS/400's
//! `abrec`. It says whose the disk is (the host's identity), which pool
//! it is in, the pool's protection and layout, and every unit of the
//! pool with its state, serial number and location code, so that a
//! pool is assembled from its disks alone and a unit that is not there
//! is still known by where it was.
//!
//! The copies on the disks of one pool agree while they run together;
//! the one with the highest generation is the pool's.

use abrec::{SLOT_BYTES, Spec};
use uuid::Uuid;

use crate::layout::record_type;
use crate::names::PoolDiskNames;

/// The payload's version.
const VERSION: u8 = 1;
/// Bytes before the units.
const HEAD_LEN: usize = 104;
/// Bytes of a unit.
const UNIT_LEN: usize = 80;
const TEXT_SERIAL: usize = 20;
const TEXT_LOCATION: usize = 32;
/// The most units a label holds: members and hot spares.
pub const MAX_UNITS: usize = 32;
/// A pool's name's most characters.
pub const NAME_MAX: usize = 10;
/// A sync mark that says the parity is computed.
const NO_SYNC: u64 = u64::MAX;

/// The label as `abrec` sees it.
pub fn spec(names: &PoolDiskNames) -> Spec {
    Spec {
        magic: names.label_magic,
        version: 1,
        type_guid: record_type(names),
        slot_bytes: SLOT_BYTES,
        min_payload: HEAD_LEN,
    }
}

/// A pool's protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protection {
    /// One disk.
    None,
    /// Two disks, each a copy.
    Mirrored,
    /// Three or more, one parity page a stripe.
    Raid5,
    /// Four or more, two parity pages a stripe.
    Raid6,
}

impl Protection {
    fn as_u8(self) -> u8 {
        match self {
            Protection::None => 0,
            Protection::Mirrored => 1,
            Protection::Raid5 => 2,
            Protection::Raid6 => 3,
        }
    }

    fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Protection::None,
            1 => Protection::Mirrored,
            2 => Protection::Raid5,
            3 => Protection::Raid6,
            _ => return None,
        })
    }

    /// As it is written and shown: `none`, `mirrored`, `raid5`, `raid6`.
    pub fn word(self) -> &'static str {
        match self {
            Protection::None => "none",
            Protection::Mirrored => "mirrored",
            Protection::Raid5 => "raid5",
            Protection::Raid6 => "raid6",
        }
    }

    /// From its word.
    pub fn from_word(word: &str) -> Option<Self> {
        [
            Protection::None,
            Protection::Mirrored,
            Protection::Raid5,
            Protection::Raid6,
        ]
        .into_iter()
        .find(|p| p.word() == word)
    }

    /// How many units may fail with the pool still whole.
    pub fn tolerates(self) -> usize {
        match self {
            Protection::None => 0,
            Protection::Mirrored | Protection::Raid5 => 1,
            Protection::Raid6 => 2,
        }
    }

    /// The least and the most member disks.
    pub fn members(self) -> (usize, usize) {
        match self {
            Protection::None => (1, 1),
            Protection::Mirrored => (2, 2),
            Protection::Raid5 => (3, protect::parity::MAX_MEMBERS),
            Protection::Raid6 => (4, protect::parity::MAX_MEMBERS),
        }
    }
}

/// A unit's state, as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    /// In step.
    Active,
    /// Being rebuilt: in step below the mark, in blocks.
    Rebuilding(u64),
    /// A mirror's member being brought back in step.
    Resuming,
    /// A mirror's member not written to: its data is stale.
    Suspended,
    /// Taken out of service: failing I/O.
    Failed,
    /// Not found.
    Missing,
    /// A hot spare, waiting.
    Spare,
}

impl UnitState {
    fn code(self) -> (u8, u64) {
        match self {
            UnitState::Active => (1, 0),
            UnitState::Rebuilding(mark) => (2, mark),
            UnitState::Resuming => (3, 0),
            UnitState::Suspended => (4, 0),
            UnitState::Failed => (5, 0),
            UnitState::Missing => (6, 0),
            UnitState::Spare => (7, 0),
        }
    }

    fn from_code(code: u8, mark: u64) -> Option<Self> {
        Some(match code {
            1 => UnitState::Active,
            2 => UnitState::Rebuilding(mark),
            3 => UnitState::Resuming,
            4 => UnitState::Suspended,
            5 => UnitState::Failed,
            6 => UnitState::Missing,
            7 => UnitState::Spare,
            _ => return None,
        })
    }

    /// As the displays word it.
    pub fn word(self) -> &'static str {
        match self {
            UnitState::Active => "Active",
            UnitState::Rebuilding(_) => "Rebuilding",
            UnitState::Resuming => "Resuming",
            UnitState::Suspended => "Suspended",
            UnitState::Failed => "Failed",
            UnitState::Missing => "Missing",
            UnitState::Spare => "Hot spare",
        }
    }

    /// Whether the unit holds the pool's data now.
    pub fn in_step(self) -> bool {
        self == UnitState::Active
    }
}

/// One unit of a pool: a member, or a hot spare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    /// The GPT GUID of its member partition: how it is found.
    pub partition: Uuid,
    /// Its state when the label was written.
    pub state: UnitState,
    /// Its disk's serial number.
    pub serial: String,
    /// Its disk's location code.
    pub location: String,
}

/// The label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolLabel {
    /// Whose the disk is: the host's identity.
    pub host: [u8; 16],
    /// The pool.
    pub pool: Uuid,
    /// The pool's name.
    pub name: String,
    /// Its protection.
    pub protection: Protection,
    /// Blocks (4096 bytes) the pool presents.
    pub set_blocks: u64,
    /// Blocks of each member the set was made over.
    pub member_blocks: u64,
    /// A parity set's layout: blocks and head, as it was planned.
    pub parity: (u64, u64),
    /// A parity set's synchronization mark: `None` when the parity is
    /// computed.
    pub sync: Option<u64>,
    /// Whether the pool was stopped in order.
    pub clean: bool,
    /// When this copy was written, in seconds since the epoch.
    pub written: u64,
    /// The copy's generation: the highest is the pool's.
    pub generation: u64,
    /// The members in order (unit 1 first), then the hot spares.
    pub units: Vec<Unit>,
}

/// Whether `name` is a pool's name: 1 to 10 of `A` to `Z`, `0` to `9`
/// and `_`, a letter first.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && name.as_bytes()[0].is_ascii_uppercase()
}

fn put_text(out: &mut [u8], text: &str) {
    let n = text.len().min(out.len());
    out[..n].copy_from_slice(&text.as_bytes()[..n]);
}

fn text(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).ok()
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

impl PoolLabel {
    /// The payload bytes.
    pub fn encode(&self) -> Vec<u8> {
        let count = self.units.len().min(MAX_UNITS);
        let mut out = vec![0u8; HEAD_LEN + count * UNIT_LEN];
        out[0] = VERSION;
        out[1] = self.protection.as_u8();
        out[2] = u8::from(self.clean);
        #[allow(clippy::cast_possible_truncation)] // MAX_UNITS
        {
            out[3] = count as u8;
        }
        out[4..20].copy_from_slice(&self.host);
        out[20..36].copy_from_slice(self.pool.as_bytes());
        put_text(&mut out[36..36 + NAME_MAX], &self.name);
        let words = [
            self.set_blocks,
            self.member_blocks,
            self.parity.0,
            self.parity.1,
            self.sync.unwrap_or(NO_SYNC),
            self.written,
            self.generation,
        ];
        for (i, w) in words.iter().enumerate() {
            out[48 + i * 8..56 + i * 8].copy_from_slice(&w.to_le_bytes());
        }
        for (i, u) in self.units.iter().take(count).enumerate() {
            let at = HEAD_LEN + i * UNIT_LEN;
            let (code, mark) = u.state.code();
            out[at..at + 16].copy_from_slice(u.partition.as_bytes());
            out[at + 16] = code;
            out[at + 18..at + 26].copy_from_slice(&mark.to_le_bytes());
            put_text(&mut out[at + 26..at + 26 + TEXT_SERIAL], &u.serial);
            let loc = at + 26 + TEXT_SERIAL;
            put_text(&mut out[loc..loc + TEXT_LOCATION], &u.location);
        }
        out
    }

    /// From payload bytes; `None` when they are not a label.
    pub fn decode(payload: &[u8]) -> Option<Self> {
        if payload.len() < HEAD_LEN || payload[0] != VERSION || payload[2] > 1 {
            return None;
        }
        let count = usize::from(payload[3]);
        if count == 0 || count > MAX_UNITS || payload.len() < HEAD_LEN + count * UNIT_LEN {
            return None;
        }
        let name = text(&payload[36..36 + NAME_MAX])?;
        if !valid_name(&name) {
            return None;
        }
        let word = |i: usize| u64_at(payload, 48 + i * 8);
        let mut units = Vec::with_capacity(count);
        for i in 0..count {
            let u = &payload[HEAD_LEN + i * UNIT_LEN..][..UNIT_LEN];
            let loc = 26 + TEXT_SERIAL;
            units.push(Unit {
                partition: Uuid::from_slice(&u[..16]).ok()?,
                state: UnitState::from_code(u[16], u64_at(u, 18))?,
                serial: text(&u[26..26 + TEXT_SERIAL])?,
                location: text(&u[loc..loc + TEXT_LOCATION])?,
            });
        }
        let mut host = [0u8; 16];
        host.copy_from_slice(&payload[4..20]);
        let sync = word(4);
        Some(Self {
            host,
            pool: Uuid::from_slice(&payload[20..36]).ok()?,
            name,
            protection: Protection::from_u8(payload[1])?,
            set_blocks: word(0),
            member_blocks: word(1),
            parity: (word(2), word(3)),
            sync: (sync != NO_SYNC).then_some(sync),
            clean: payload[2] == 1,
            written: word(5),
            generation: word(6),
            units,
        })
    }

    /// The members, in unit order: every unit that is not a spare.
    pub fn members(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter(|u| u.state != UnitState::Spare)
    }

    /// The hot spares.
    pub fn spares(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter(|u| u.state == UnitState::Spare)
    }

    /// The unit on the partition `partition`, and its number from 1.
    pub fn unit_of(&self, partition: Uuid) -> Option<(usize, &Unit)> {
        self.units
            .iter()
            .enumerate()
            .find(|(_, u)| u.partition == partition)
            .map(|(i, u)| (i + 1, u))
    }

    /// Members not in step: failed, missing, being rebuilt or resumed.
    pub fn out_of_step(&self) -> usize {
        self.members().filter(|u| !u.state.in_step()).count()
    }

    /// Whether more members are out of step than the protection covers.
    pub fn lost(&self) -> bool {
        self.out_of_step() > self.protection.tolerates()
    }
}

/// Of the labels found on disks, the newest of each pool: the one
/// with the highest generation, by pool, in the order first seen.
pub fn newest(found: &[PoolLabel]) -> Vec<PoolLabel> {
    let mut pools: Vec<PoolLabel> = Vec::new();
    for l in found {
        match pools.iter_mut().find(|p| p.pool == l.pool) {
            Some(p) if l.generation > p.generation => *p = l.clone(),
            Some(_) => {}
            None => pools.push(l.clone()),
        }
    }
    pools
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn label(protection: Protection, members: usize) -> PoolLabel {
        let mut units: Vec<Unit> = (0..members)
            .map(|i| Unit {
                partition: Uuid::from_u128(100 + i as u128),
                state: UnitState::Active,
                serial: format!("SER{i}"),
                location: format!("P0000-B00-D0{i}-F0-U0"),
            })
            .collect();
        units.push(Unit {
            partition: Uuid::from_u128(999),
            state: UnitState::Spare,
            serial: "SPARE".into(),
            location: String::new(),
        });
        PoolLabel {
            host: [7; 16],
            pool: Uuid::from_u128(42),
            name: "POOL1".into(),
            protection,
            set_blocks: 1_000_000,
            member_blocks: 500_000,
            parity: (0, 0),
            sync: None,
            clean: true,
            written: 1_794_478_923,
            generation: 3,
            units,
        }
    }

    #[test]
    fn the_label_round_trips_with_every_state() {
        let mut l = label(Protection::Raid6, 4);
        l.units[1].state = UnitState::Rebuilding(12_345);
        l.units[2].state = UnitState::Missing;
        l.sync = Some(77);
        l.parity = (166_000, 0);
        let back = PoolLabel::decode(&l.encode()).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.members().count(), 4);
        assert_eq!(back.spares().count(), 1);
        assert_eq!(back.unit_of(Uuid::from_u128(101)).unwrap().0, 2);
        assert_eq!(back.out_of_step(), 2);
        assert!(!back.lost(), "RAID 6 holds with two out");
        let mut r5 = l.clone();
        r5.protection = Protection::Raid5;
        assert!(r5.lost());
        // Encoded within a copy's slot, with room to spare.
        assert!(l.encode().len() + abrec::HEAD_LEN < 32 * 1024);
    }

    #[test]
    fn what_is_not_a_label_is_refused() {
        let bytes = label(Protection::Mirrored, 2).encode();
        assert!(PoolLabel::decode(&bytes[..50]).is_none());
        // The version, the protection, a name that starts with a digit,
        // a unit's state, the count of units.
        for (at, value) in [(0, 9), (1, 9), (36, b'1'), (HEAD_LEN + 16, 0), (3, 0)] {
            let mut changed = bytes.clone();
            changed[at] = value;
            assert!(PoolLabel::decode(&changed).is_none(), "byte {at}");
        }
    }

    #[test]
    fn the_newest_copy_of_each_pool_is_its_label() {
        let a = label(Protection::Mirrored, 2);
        let mut a_newer = a.clone();
        a_newer.generation = 9;
        let mut b = label(Protection::None, 1);
        b.pool = Uuid::from_u128(43);
        let pools = newest(&[a.clone(), b.clone(), a_newer.clone(), a]);
        assert_eq!(pools, [a_newer, b]);
    }

    #[test]
    fn names_and_protections_are_as_written() {
        assert!(valid_name("POOL_01"));
        assert!(!valid_name("pool"));
        assert!(!valid_name("1POOL"));
        assert!(!valid_name("POOL_NAME_TOO"));
        for p in ["none", "mirrored", "raid5", "raid6"] {
            assert_eq!(Protection::from_word(p).unwrap().word(), p);
        }
        assert_eq!(Protection::Raid5.members(), (3, 18));
        assert_eq!(Protection::Raid6.tolerates(), 2);
    }
}
