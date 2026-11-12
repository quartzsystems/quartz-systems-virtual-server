// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Host reference codes. One table, no scattered strings.
//!
//! Every code is eight characters, a family and six digits. The host's
//! families end in `2` (docs/REFCODES.md), which no QS/400 family does:
//! `C2` host IPL and running, `A2` attention, `B2` fatal, `D2` power down.

#![forbid(unsafe_code)]

/// What a code's family says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// `C2`: the host's IPL and its progress.
    Progress,
    /// `A2`: attention, the host continues.
    Attention,
    /// `B2`: fatal, the host parks.
    Fatal,
    /// `D2`: power down.
    PowerDown,
}

impl Family {
    /// The two characters a code of the family starts with.
    pub const fn prefix(self) -> &'static str {
        match self {
            Family::Progress => "C2",
            Family::Attention => "A2",
            Family::Fatal => "B2",
            Family::PowerDown => "D2",
        }
    }
}

/// One reference code.
#[derive(Debug, PartialEq, Eq)]
pub struct RefCode {
    /// Eight-character code.
    pub code: &'static str,
    /// Short text.
    pub text: &'static str,
    /// Family.
    pub family: Family,
}

const fn code(code: &'static str, text: &'static str, family: Family) -> RefCode {
    RefCode { code, text, family }
}

/// PID 1 started the host IPL.
pub const IPL_STARTED: RefCode = code("C2001000", "Host IPL started", Family::Progress);
/// The host is running: posted on the serial port every five seconds.
pub const HEARTBEAT: RefCode = code("C2001005", "Host running", Family::Progress);
/// PID 1 panicked and parked.
pub const PANIC: RefCode = code("B2001000", "Host PID 1 failed", Family::Fatal);

/// Every code, in posting order.
pub const ALL: &[&RefCode] = &[&IPL_STARTED, &HEARTBEAT, &PANIC];

/// The word a posted line starts with.
pub const WORD: &str = "REF";

impl RefCode {
    /// `REF C2001000`, what a reader of the console looks for.
    pub fn tag(&self) -> String {
        format!("{WORD} {}", self.code)
    }

    /// The line as posted: `REF C2001000 Host IPL started`.
    pub fn line(&self) -> String {
        format!("{WORD} {} {}", self.code, self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The families of QS/400's reference codes (`lic/slic/src/src.rs` there).
    const QS400_FAMILIES: &[&str] = &["C1", "C6", "A6", "B1", "D1"];

    #[test]
    fn codes_are_well_formed_and_unique() {
        for (i, c) in ALL.iter().enumerate() {
            assert_eq!(c.code.len(), 8, "{}", c.code);
            assert!(c.code.starts_with(c.family.prefix()), "{}", c.code);
            assert!(
                c.code[2..].bytes().all(|b| b.is_ascii_digit()),
                "{}",
                c.code
            );
            assert!(ALL[..i].iter().all(|o| o.code != c.code), "{}", c.code);
        }
    }

    #[test]
    fn no_code_is_in_a_qs400_family() {
        for c in ALL {
            assert!(!QS400_FAMILIES.contains(&&c.code[..2]), "{}", c.code);
        }
    }

    #[test]
    fn a_line_is_the_word_the_code_and_the_text() {
        assert_eq!(IPL_STARTED.line(), "REF C2001000 Host IPL started");
        assert_eq!(HEARTBEAT.tag(), "REF C2001005");
    }
}
