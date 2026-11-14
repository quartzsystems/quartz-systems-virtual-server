// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A storage pool: one protected set of whole disks with one xfs file
//! system on it (`docs/STORAGE.md`).
//!
//! - [`names`]: what a pool disk carries that is the host's own: its
//!   partitions' types and names, its records' magics.
//! - [`layout`]: a pool disk: a GPT with a record partition and a
//!   member partition, and where each record is in the first.
//! - [`label`]: the member label every disk of a pool carries: the
//!   pool, its protection and layout, and every unit with its state.
//! - [`plan`]: a pool to be made: its protection, its capacity, the
//!   limits it must keep.
//! - [`disk`]: what each disk of the host is, by what is on it.
//!
//! The labels are the truth: the pool service's state file is made
//! again from them when it is lost.

pub mod disk;
pub mod label;
pub mod layout;
pub mod names;
pub mod plan;

pub use label::{PoolLabel, Protection, Unit, UnitState};
pub use names::PoolDiskNames;
