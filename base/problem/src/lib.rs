// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The problem log (`docs/PROBLEMS.md`). A problem is one record per
//! condition a person should look at, opened from a posted reference
//! code by the product's table (`table::Rules`). The same
//! condition seen again while its problem is open adds to its count and
//! its analysis log; a code that clears it marks it cleared; a person
//! closes it.
//!
//! - [`record`]: a problem, as kept and as the socket carries it.
//! - [`book`]: the log: what a posting opens, counts and clears, and a
//!   close; bounded, numbered without reuse.
//! - [`feed`]: the reference code history, read from where it had got
//!   to.
//! - [`proto`]: the problem service's socket.
//! - `serve` (Linux): the service.

#![forbid(unsafe_code)]

pub mod book;
pub mod feed;
pub mod proto;
pub mod record;
#[cfg(target_os = "linux")]
pub mod serve;
pub mod table;

pub use book::Book;
pub use record::{
    Filter, Line, Note, Notice, Noticed, Page, Posted, Problem, Rated, RatedPage, RatedProblem,
    Severity, Source, Status, Summary,
};
