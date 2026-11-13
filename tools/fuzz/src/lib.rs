// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's fuzzer. libFuzzer, which `cargo fuzz` runs, is C++, and
//! this tree builds no C; so the loop is here, in Rust.
//!
//! - [`cov`]: LLVM's coverage counters, when the build has them.
//! - [`mutate`]: the next input from one that was kept.
//! - [`engine`]: the loop: run, catch a panic, keep what reached new
//!   code.
//! - [`targets`]: every reader of bytes a guest controls.
//!
//! `cargo run -p build -- fuzz` builds this with the counters and runs
//! every target. A finding is a panic or an input that runs too long;
//! the input is written under `<target>/fuzz/crashes/`.
//!
//! `unsafe` lives only in `cov`.

#![deny(unsafe_code)]

pub mod cov;
pub mod engine;
pub mod mutate;
pub mod targets;
