// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition model: a partition's definition and its profiles, as
//! they are kept in state, sent over the partition manager's socket
//! and checked.
//!
//! - [`definition`]: what a partition is: ID, name, serial number,
//!   keylock, default profile, auto start.
//! - [`profile`]: what an activation gives it: processors, memory,
//!   storage, boot, and the sections later phases bring.
//! - [`units`]: processing units.
//! - [`validate`]: every error at once, each a `QVS` message.
//! - [`sample`]: a definition and two profiles that are valid, for
//!   tests and documents.

#![forbid(unsafe_code)]

pub mod definition;
pub mod profile;
pub mod sample;
pub mod units;
pub mod validate;

pub use definition::{AutoStart, Definition, Environment, Keylock};
pub use profile::{
    Boot, ConsoleMode, EthernetAdapter, IplSource, IplType, Lun, MediaDevice, Memory, PhysicalIo,
    ProcessorMode, Processors, Profile, ScsiAdapter, Sharing,
};
pub use units::Units;
pub use validate::{Limits, valid_name};
