// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Host reference codes. One table, no scattered strings.
//!
//! Every code is eight characters, a family and six digits. The host's
//! families end in `2` (docs/REFCODES.md), which no QS/400 family does:
//! `C2` host IPL and running, `A2` attention, `B2` fatal, `D2` power down.
//!
//! - [`event`]: the code of each event the base crates report.
//! - [`post`]: the poster: the console line and the history record,
//!   and how a service posts through PID 1.

#![forbid(unsafe_code)]

pub mod event;
pub mod post;

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

const fn progress(code: &'static str, text: &'static str) -> RefCode {
    RefCode {
        code,
        text,
        family: Family::Progress,
    }
}

const fn attention(code: &'static str, text: &'static str) -> RefCode {
    RefCode {
        code,
        text,
        family: Family::Attention,
    }
}

const fn power_down(code: &'static str, text: &'static str) -> RefCode {
    RefCode {
        code,
        text,
        family: Family::PowerDown,
    }
}

// C2001xxx: PID 1's own IPL, in the order it is posted.

/// PID 1 started the host IPL.
pub const IPL_STARTED: RefCode = progress("C2001000", "Host IPL started");
/// The host is running: posted on the serial port every five seconds.
pub const HEARTBEAT: RefCode = progress("C2001005", "Host running");
/// proc, sysfs, devtmpfs, the run directory and efivarfs are mounted.
pub const MOUNTED: RefCode = progress("C2001010", "Kernel file systems mounted");
/// cgroup2 is mounted.
pub const CGROUPS: RefCode = progress("C2001020", "Control groups mounted");
/// The A/B record was read.
pub const RECORD_READ: RefCode = progress("C2001030", "A/B record read");
/// `QSVSDATA` is mounted at `/data`.
pub const DATA_MOUNTED: RefCode = progress("C2001040", "Data file system mounted");
/// The system clock was set from the RTC.
pub const CLOCK_SET: RefCode = progress("C2001050", "Clock set from the RTC");
/// The seed step ran and applied nothing.
pub const SEED_CHECKED: RefCode = progress("C2001060", "Seed checked");
/// The state files were written from a seed volume.
pub const SEED_APPLIED: RefCode = progress("C2001061", "Seed applied");
/// The state files were read and the inventory written.
pub const STATE_LOADED: RefCode = progress("C2001070", "State loaded");
/// The service table was started.
pub const SERVICES_STARTED: RefCode = progress("C2001080", "Services started");
/// Every base service is ready.
pub const BASE_READY: RefCode = progress("C2001090", "Base ready");
/// The slot this IPL came from is confirmed as the default.
pub const SLOT_CONFIRMED: RefCode = progress("C2001095", "Slot confirmed");

// C2002xxx: the management network and time.

/// The management interface is up and has carrier.
pub const INTERFACE_UP: RefCode = progress("C2002010", "Management interface up");
/// The management interface has its address.
pub const ADDRESS_ACQUIRED: RefCode = progress("C2002020", "Management address acquired");
/// The clock is in step with a time server.
pub const TIME_SYNCHRONIZED: RefCode = progress("C2002030", "Time synchronized");

// A2001xxx: attention from PID 1, numbered after the IPL step.

/// The A/B record had no valid copy; a new one was written.
pub const RECORD_INVALID: RefCode = attention("A2001030", "A/B record not valid");
/// The pending slot did not confirm; the host is back on the default.
pub const SLOT_FELL_BACK: RefCode = attention("A2001035", "Pending slot not confirmed");
/// `/data` is not mounted; state is in memory only.
pub const DATA_NOT_MOUNTED: RefCode = attention("A2001040", "Data file system not mounted");
/// The RTC could not be read.
pub const CLOCK_NOT_SET: RefCode = attention("A2001050", "Clock not set from the RTC");
/// A seed volume was found and refused.
pub const SEED_INVALID: RefCode = attention("A2001060", "Seed not valid");
/// A state file was refused; its defaults are in use.
pub const STATE_INVALID: RefCode = attention("A2001070", "State file not valid");
/// A service failed five times in ten minutes and is not restarted.
pub const SERVICE_STOPPED: RefCode = attention("A2001080", "Service stopped");
/// The slot was confirmed in the record but not in the firmware.
pub const BOOT_ORDER_NOT_WRITTEN: RefCode = attention("A2001095", "Boot order not written");

// A2002xxx: attention from the management network.

/// There is no interface to manage the host through.
pub const NO_INTERFACE: RefCode = attention("A2002005", "No management interface");
/// The management interface has no carrier.
pub const NO_CARRIER: RefCode = attention("A2002010", "No carrier on the management interface");
/// The management interface has no address.
pub const NO_ADDRESS: RefCode = attention("A2002020", "No management address");

/// PID 1 panicked and parked.
pub const PANIC: RefCode = RefCode {
    code: "B2001000",
    text: "Host PID 1 failed",
    family: Family::Fatal,
};

/// The orderly shutdown began.
pub const SHUTDOWN_STARTED: RefCode = power_down("D2001000", "Host shutdown started");
/// Services are stopped and `/data` unmounted: power off follows.
pub const POWER_OFF: RefCode = power_down("D2001090", "Host powering off");
/// Services are stopped and `/data` unmounted: a restart follows.
pub const RESTART: RefCode = power_down("D2001091", "Host restarting");

/// Every code, in table order.
pub const ALL: &[&RefCode] = &[
    &IPL_STARTED,
    &HEARTBEAT,
    &MOUNTED,
    &CGROUPS,
    &RECORD_READ,
    &DATA_MOUNTED,
    &CLOCK_SET,
    &SEED_CHECKED,
    &SEED_APPLIED,
    &STATE_LOADED,
    &SERVICES_STARTED,
    &BASE_READY,
    &SLOT_CONFIRMED,
    &INTERFACE_UP,
    &ADDRESS_ACQUIRED,
    &TIME_SYNCHRONIZED,
    &RECORD_INVALID,
    &SLOT_FELL_BACK,
    &DATA_NOT_MOUNTED,
    &CLOCK_NOT_SET,
    &SEED_INVALID,
    &STATE_INVALID,
    &SERVICE_STOPPED,
    &BOOT_ORDER_NOT_WRITTEN,
    &NO_INTERFACE,
    &NO_CARRIER,
    &NO_ADDRESS,
    &PANIC,
    &SHUTDOWN_STARTED,
    &POWER_OFF,
    &RESTART,
];

/// The word a posted line starts with.
pub const WORD: &str = "REF";

/// The code spelled `code`, when the table has it.
pub fn find(code: &str) -> Option<&'static RefCode> {
    ALL.iter().copied().find(|c| c.code == code)
}

impl RefCode {
    /// `REF C2001000`, what a reader of the console looks for.
    pub fn tag(&self) -> String {
        format!("{WORD} {}", self.code)
    }

    /// The line as posted: `REF C2001000 Host IPL started`.
    pub fn line(&self) -> String {
        format!("{WORD} {} {}", self.code, self.text)
    }

    /// The text with what this posting adds: `Service stopped: netd`.
    pub fn text_with(&self, detail: &str) -> String {
        if detail.is_empty() {
            self.text.to_string()
        } else {
            format!("{}: {detail}", self.text)
        }
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
            assert!(!c.text.is_empty() && !c.text.contains(':'), "{}", c.code);
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
        assert_eq!(SERVICE_STOPPED.text_with("netd"), "Service stopped: netd");
        assert_eq!(BASE_READY.text_with(""), "Base ready");
        assert_eq!(find("D2001090"), Some(&POWER_OFF));
        assert_eq!(find("C1001000"), None);
    }

    /// docs/REFCODES.md lists every code with its text, and no other.
    #[test]
    fn the_document_lists_the_table() {
        let doc = include_str!("../../../docs/REFCODES.md");
        for c in ALL {
            let row = format!("| `{}` | {} |", c.code, c.text);
            assert!(doc.contains(&row), "docs/REFCODES.md lacks: {row}");
        }
        let is_code_row = |l: &&str| {
            l.starts_with("| `")
                && l.get(11..12) == Some("`")
                && l[5..11].bytes().all(|b| b.is_ascii_digit())
        };
        assert_eq!(doc.lines().filter(is_code_row).count(), ALL.len());
    }
}
