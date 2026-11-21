// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the product gives the problem log: its table of conditions,
//! each attention or fatal code and whether it opens a problem, how
//! grave each is, and the messages the log refuses with. The words and
//! the codes are the product's; the rules are these.

use msg::Message;

use crate::record::Severity;

/// How a condition opens a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opening {
    /// What describes it: its first-level text is the problem's
    /// description, its second-level text what to do.
    pub message: &'static Message,
    /// The kinds of resource its message names, in order
    /// (`supervise::control::about`).
    pub resources: &'static [&'static str],
    /// The progress codes whose posting marks it cleared.
    pub cleared_by: &'static [&'static str],
    /// The problem's code is the system's own, which the posting names,
    /// not the code that posted it.
    pub system_code: bool,
}

/// One attention or fatal code of the product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Condition {
    /// The code.
    pub code: &'static str,
    /// How it opens a problem; none when it does not.
    pub opens: Option<Opening>,
    /// Why it opens none: where a person sees it instead.
    pub note: &'static str,
}

/// The messages the log refuses with, the product's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Messages {
    /// No problem of the number (`&1`).
    pub no_problem: &'static Message,
    /// Closed already (`&1`, by `&2`).
    pub closed: &'static Message,
    /// A close while the condition holds (`&1`).
    pub holds: &'static Message,
    /// The log could not be written.
    pub not_kept: &'static Message,
    /// A note that is not one (`&1` bytes, `&2` the most).
    pub note_invalid: &'static Message,
}

/// The product's rules for its problem log.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    /// Every attention and fatal code, in order.
    pub conditions: &'static [Condition],
    /// How grave a problem the code opened is.
    pub severity: fn(&str) -> Severity,
    /// The messages it refuses with.
    pub messages: Messages,
}

impl Rules {
    /// The condition of `code`.
    #[must_use]
    pub fn condition(&self, code: &str) -> Option<&'static Condition> {
        self.conditions.iter().find(|c| c.code == code)
    }

    /// Each condition the progress code `code` clears.
    pub fn cleared<'a>(&self, code: &'a str) -> impl Iterator<Item = &'static Condition> + use<'a> {
        self.conditions.iter().filter(move |c| {
            c.opens
                .as_ref()
                .is_some_and(|o| o.cleared_by.contains(&code))
        })
    }
}

#[cfg(test)]
pub(crate) mod sample {
    //! A small table of an appliance of no product's, for the tests.

    use super::*;

    macro_rules! m {
        ($name:ident, $id:literal, $text:literal) => {
            pub const $name: Message = Message {
                id: $id,
                text: $text,
                help: "Look at it.",
            };
        };
    }
    m!(NO_PROBLEM, "XMP5001", "There is no problem &1.");
    m!(CLOSED, "XMP5002", "Problem &1 was closed already, by &2.");
    m!(HOLDS, "XMP5003", "The condition of problem &1 still holds.");
    m!(NOT_KEPT, "XMP5004", "The problem log could not be written.");
    m!(NOTE_INVALID, "XMP5005", "A note of &1 bytes is not valid.");
    m!(
        UNIT_FAILED,
        "XMP5130",
        "Disk unit &1 of storage pool &2 failed or is missing."
    );
    m!(
        NOT_RESPONDING,
        "XMP5140",
        "Partition &1 is not responding on its service channel."
    );
    m!(
        TERMINAL,
        "XMP5142",
        "Partition &1 is held after its system posted a terminal code: &2."
    );

    /// The rows the tests use.
    pub const CONDITIONS: &[Condition] = &[
        Condition {
            code: "A2005000",
            opens: None,
            note: "said at the activation",
        },
        Condition {
            code: "A2007020",
            opens: Some(Opening {
                message: &UNIT_FAILED,
                resources: &["disk-unit", "storage-pool"],
                cleared_by: &["C2007035"],
                system_code: false,
            }),
            note: "",
        },
        Condition {
            code: "A2009020",
            opens: Some(Opening {
                message: &NOT_RESPONDING,
                resources: &["partition"],
                cleared_by: &["C2009020"],
                system_code: false,
            }),
            note: "",
        },
        Condition {
            code: "A2009055",
            opens: Some(Opening {
                message: &TERMINAL,
                resources: &["partition"],
                cleared_by: &[],
                system_code: true,
            }),
            note: "",
        },
    ];

    fn severity(code: &str) -> Severity {
        if code.starts_with('B') {
            Severity::Terminal
        } else {
            Severity::Attention
        }
    }

    /// The sample's rules.
    pub const RULES: Rules = Rules {
        conditions: CONDITIONS,
        severity,
        messages: Messages {
            no_problem: &NO_PROBLEM,
            closed: &CLOSED,
            holds: &HOLDS,
            not_kept: &NOT_KEPT,
            note_invalid: &NOTE_INVALID,
        },
    };
}
