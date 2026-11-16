// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The problem log (`docs/PROBLEMS.md`). A problem is one record per
//! condition a person should look at, opened from a posted reference
//! code by the catalogue's table (`qvsmsg::problems`). The same
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

pub use book::Book;
pub use record::{Filter, Line, Notice, Noticed, Page, Posted, Problem, Source, Status, Summary};

#[cfg(test)]
mod tests {
    use qvsmsg::problems::{CONDITIONS, condition, kind as table};
    use refcode::Family;
    use supervise::control::about;

    /// Every attention and fatal code has its row, and every code a row
    /// names is one of the table's.
    #[test]
    fn the_table_holds_every_code_of_the_host() {
        for c in refcode::ALL {
            if matches!(c.family, Family::Attention | Family::Fatal) {
                assert!(condition(c.code).is_some(), "{} has no row", c.code);
            }
        }
        for row in CONDITIONS {
            let code = refcode::find(row.code).unwrap_or_else(|| panic!("{}", row.code));
            assert!(matches!(code.family, Family::Attention | Family::Fatal));
            if let Some(o) = &row.opens {
                for clear in o.cleared_by {
                    let c = refcode::find(clear).unwrap_or_else(|| panic!("{clear}"));
                    assert_eq!(c.family, Family::Progress, "{clear}");
                }
            }
        }
    }

    /// The table's kinds are the posting's.
    #[test]
    fn the_kinds_are_the_postings() {
        let pairs = [
            (table::DISK_UNIT, about::DISK_UNIT),
            (table::STORAGE_POOL, about::STORAGE_POOL),
            (table::VIRTUAL_DISK, about::VIRTUAL_DISK),
            (table::NETWORK_INTERFACE, about::NETWORK_INTERFACE),
            (table::VIRTUAL_SWITCH, about::VIRTUAL_SWITCH),
            (table::PARTITION, about::PARTITION),
            (table::CODE_SLOT, about::CODE_SLOT),
            (table::SERVICE, about::SERVICE),
        ];
        for (a, b) in pairs {
            assert_eq!(a, b);
        }
    }
}
