// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A virtual tape volume.
//!
//! - [`aws`]: the image, in the AWSTAPE format: each block of data and
//!   each filemark is a six-byte header and its bytes, one after the
//!   other, and the end of the file is the end of data. Other tools
//!   read it (`docs/STORAGE.md`).
//! - [`medium`]: a volume positioned, read and written as a drive does
//!   it: blocks, filemarks, spacing, locating, its capacity and early
//!   warning.
//! - [`volume`]: what is kept beside the image: the volume identifier,
//!   its capacity and whether it is write-protected.
//! - [`passthrough`]: a physical drive passed through: the allow-list,
//!   the frames between the VMM and the relay, the drive.
//!
//! The image is written by a guest, so a damaged one is read up to its
//! last whole header and ends there; nothing here panics on it or
//! allocates by a length it names beyond the fixed bounds.

pub mod aws;
pub mod medium;
pub mod passthrough;
pub mod volume;

pub use medium::{Medium, Read, Stop, Store, TapeError};
pub use volume::Volume;
