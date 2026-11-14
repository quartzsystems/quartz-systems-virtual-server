// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A pool to be made: the disks chosen are checked, every problem at
//! once, and the capacity is worked out before anything is written.
//!
//! The limits are QS/400's `protect` and `xfsfmt` as handoff 15 states
//! them (`docs/STORAGE.md`): a parity set has 18 members at most; a
//! file system is formatted at 64 TiB at most; a set cannot grow, so a
//! pool's size is fixed when it is made.

use protect::parity::{Geometry, Level};
use protect::set::ParitySet;

use crate::label::Protection;
use crate::layout::{self, BLOCK};
use crate::names::PoolDiskNames;

/// The largest pool: what `xfsfmt` formats.
pub const MAX_POOL_BYTES: u64 = 64 << 40;
/// The most hot spares a pool has.
pub const MAX_SPARES: usize = 8;
/// Memory the pool service holds for a mirrored pool, a TiB: the
/// write-intent map's bits and a resume's chunk map.
pub const MIRROR_MEMORY_PER_TIB: u64 = 136 * 1024;

/// A disk offered for a pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Its kernel name.
    pub disk: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// Its logical block size.
    pub logical_block: u32,
    /// Why it cannot be taken, when it cannot: the load source, in
    /// another pool, a hot spare, foreign data not confirmed.
    pub taken: Option<String>,
}

/// Why a pool cannot be made of what was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The protection takes another number of members.
    Members {
        /// The protection.
        protection: Protection,
        /// The least and the most it takes.
        range: (usize, usize),
        /// How many were chosen.
        chosen: usize,
    },
    /// A disk is chosen twice.
    Twice(String),
    /// A disk cannot be taken.
    Taken(String, String),
    /// A disk is smaller than a pool disk can be.
    TooSmall(String, u64),
    /// The disks' logical block sizes differ.
    BlockSizes(Vec<(String, u32)>),
    /// A hot spare is smaller than the smallest member.
    SpareTooSmall(String),
    /// More hot spares than a pool has.
    TooManySpares(usize),
    /// A pool with no protection has no use for a hot spare.
    SpareUnprotected,
    /// The pool would be larger than a file system is formatted.
    TooLarge(u64),
}

/// A pool as it would be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Its protection.
    pub protection: Protection,
    /// The member disks, in unit order.
    pub members: Vec<String>,
    /// The hot spares.
    pub spares: Vec<String>,
    /// Blocks of each member partition the set is made over.
    pub member_blocks: u64,
    /// A parity set's layout.
    pub geometry: Option<Geometry>,
    /// Blocks the pool presents.
    pub set_blocks: u64,
}

impl Plan {
    /// Bytes the pool presents.
    pub fn capacity(&self) -> u64 {
        self.set_blocks * BLOCK
    }
}

/// The member partition's blocks on a disk of `bytes`.
pub fn member_blocks(names: &PoolDiskNames, bytes: u64) -> Option<u64> {
    let l = layout::layout(names, bytes).ok()?;
    Some(l.parts.get(1)?.part.bytes() / BLOCK)
}

fn level(p: Protection) -> Option<Level> {
    match p {
        Protection::Raid5 => Some(Level::Raid5),
        Protection::Raid6 => Some(Level::Raid6),
        Protection::None | Protection::Mirrored => None,
    }
}

/// Blocks a set of `n` members of `smallest` blocks presents, and a
/// parity set's layout.
fn capacity(protection: Protection, n: usize, smallest: u64) -> Option<(u64, Option<Geometry>)> {
    match level(protection) {
        None => Some((smallest, None)),
        Some(level) => {
            let geo = ParitySet::plan(level, n, smallest, 0).ok()?;
            Some((geo.blocks * geo.w() as u64 * geo.n as u64, Some(geo)))
        }
    }
}

/// Check a pool of `members` with `spares` and `protection`, and plan
/// it: every problem, or the plan.
pub fn plan(
    names: &PoolDiskNames,
    protection: Protection,
    members: &[Candidate],
    spares: &[Candidate],
) -> Result<Plan, Vec<Problem>> {
    let mut problems = Vec::new();
    let range = protection.members();
    if !(range.0..=range.1).contains(&members.len()) {
        problems.push(Problem::Members {
            protection,
            range,
            chosen: members.len(),
        });
    }
    if spares.len() > MAX_SPARES {
        problems.push(Problem::TooManySpares(spares.len()));
    }
    if protection == Protection::None && !spares.is_empty() {
        problems.push(Problem::SpareUnprotected);
    }
    let all: Vec<&Candidate> = members.iter().chain(spares).collect();
    for (i, c) in all.iter().enumerate() {
        if all[..i].iter().any(|o| o.disk == c.disk) {
            problems.push(Problem::Twice(c.disk.clone()));
        }
        if let Some(why) = &c.taken {
            problems.push(Problem::Taken(c.disk.clone(), why.clone()));
        }
        if member_blocks(names, c.bytes).is_none() {
            problems.push(Problem::TooSmall(c.disk.clone(), c.bytes));
        }
    }
    let first = all.first().map(|c| c.logical_block);
    if all.iter().any(|c| Some(c.logical_block) != first) {
        problems.push(Problem::BlockSizes(
            all.iter()
                .map(|c| (c.disk.clone(), c.logical_block))
                .collect(),
        ));
    }
    let smallest = members
        .iter()
        .filter_map(|c| member_blocks(names, c.bytes))
        .min()
        .unwrap_or(0);
    for s in spares {
        if member_blocks(names, s.bytes).is_some_and(|b| b < smallest) {
            problems.push(Problem::SpareTooSmall(s.disk.clone()));
        }
    }
    let planned = if problems.is_empty() {
        capacity(protection, members.len(), smallest)
    } else {
        None
    };
    if let Some((set_blocks, _)) = planned
        && set_blocks * BLOCK > MAX_POOL_BYTES
    {
        problems.push(Problem::TooLarge(set_blocks * BLOCK));
    }
    match planned {
        Some((set_blocks, geometry)) if problems.is_empty() => Ok(Plan {
            protection,
            members: members.iter().map(|c| c.disk.clone()).collect(),
            spares: spares.iter().map(|c| c.disk.clone()).collect(),
            member_blocks: smallest,
            geometry,
            set_blocks,
        }),
        _ => {
            if problems.is_empty() {
                problems.push(Problem::TooSmall(String::new(), smallest * BLOCK));
            }
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &PoolDiskNames = &PoolDiskNames::DEMO;
    const GIB: u64 = 1 << 30;

    fn disk(name: &str, gib: u64) -> Candidate {
        Candidate {
            disk: name.into(),
            bytes: gib * GIB,
            logical_block: 512,
            taken: None,
        }
    }

    #[test]
    fn each_protection_has_its_capacity() {
        let one = plan(NAMES, Protection::None, &[disk("sdb", 10)], &[]).unwrap();
        let m = member_blocks(NAMES, 10 * GIB).unwrap();
        assert_eq!(one.set_blocks, m);
        let two = plan(
            NAMES,
            Protection::Mirrored,
            &[disk("sdb", 10), disk("sdc", 12)],
            &[disk("sdd", 10)],
        )
        .unwrap();
        assert_eq!(two.set_blocks, m, "the smaller of the two");
        assert_eq!(two.spares, ["sdd"]);
        let r5 = plan(
            NAMES,
            Protection::Raid5,
            &[disk("sdb", 10), disk("sdc", 10), disk("sdd", 10)],
            &[],
        )
        .unwrap();
        // Two disks' worth, less the logs.
        assert!(r5.capacity() > 19 * GIB && r5.capacity() < 20 * GIB);
        assert!(r5.geometry.is_some());
        let r6 = plan(
            NAMES,
            Protection::Raid6,
            &[disk("a", 10), disk("b", 10), disk("c", 10), disk("d", 10)],
            &[],
        )
        .unwrap();
        assert!(r6.capacity() > 19 * GIB && r6.capacity() < 20 * GIB);
    }

    #[test]
    fn every_problem_is_given_at_once() {
        let mut lsrc = disk("sda", 10);
        lsrc.taken = Some("the load source".into());
        let mut odd = disk("sdc", 10);
        odd.logical_block = 4096;
        let problems = plan(
            NAMES,
            Protection::Raid5,
            &[lsrc, disk("sdb", 10), odd, disk("sdb", 10)],
            &[disk("tiny", 1)],
        )
        .unwrap_err();
        assert!(problems.contains(&Problem::Taken("sda".into(), "the load source".into())));
        assert!(problems.contains(&Problem::Twice("sdb".into())));
        assert!(problems.iter().any(|p| matches!(p, Problem::BlockSizes(_))));
        assert!(problems.contains(&Problem::SpareTooSmall("tiny".into())));
        let few = plan(NAMES, Protection::Raid6, &[disk("a", 10)], &[]).unwrap_err();
        assert_eq!(
            few,
            [Problem::Members {
                protection: Protection::Raid6,
                range: (4, 18),
                chosen: 1
            }]
        );
        let spare = plan(NAMES, Protection::None, &[disk("a", 10)], &[disk("b", 10)]).unwrap_err();
        assert_eq!(spare, [Problem::SpareUnprotected]);
        let small = plan(NAMES, Protection::None, &[disk("a", 0)], &[]).unwrap_err();
        assert!(matches!(small[0], Problem::TooSmall(_, _)));
    }

    #[test]
    fn a_pool_larger_than_a_file_system_is_refused() {
        let big: Vec<Candidate> = (0..10).map(|i| disk(&format!("d{i}"), 8 << 10)).collect();
        let problems = plan(NAMES, Protection::Raid5, &big, &[]).unwrap_err();
        assert!(matches!(problems[0], Problem::TooLarge(_)));
    }
}
