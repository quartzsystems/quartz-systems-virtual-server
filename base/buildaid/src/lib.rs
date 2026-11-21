// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a Quartz appliance's build orchestrator does beside QS/400's
//! `buildkit`, called with the product's `Product` and its names: the
//! checks of its tree, its release, its update image and its fuzzer.
//!
//! - [`header`]: every `.rs` file starts with the license header.
//! - [`wording`]: no other vendor's product name or mark in any file
//!   git tracks, and every panel's help is whole.
//! - [`release`]: the release keys, a development key refused, Secure
//!   Boot signing of the UKIs, the release directory's sums and the
//!   stream's index signed, the release built twice and compared.
//! - [`updimg`]: the update image of what was built, in the product's
//!   format.
//! - [`fuzz`]: the product's fuzzer built with coverage counters and
//!   every target run.
//! - [`catalogue`]: every code with its meaning, cause and recovery and
//!   every message with both texts, as one file a console reads; and
//!   the same join audited.

#![forbid(unsafe_code)]

pub mod catalogue;
pub mod fuzz;
pub mod header;
pub mod release;
pub mod updimg;
pub mod wording;
