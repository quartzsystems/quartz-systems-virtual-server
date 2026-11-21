// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A problem: its summary, the resources it is about and its analysis
//! log, as kept on disk (JSON) and as the socket carries it (wire).
//! The fields are QSI's service family's (`docs/QSI.md`, service), in
//! the host's forms.

use serde::{Deserialize, Serialize};
pub use supervise::control::About;

/// Whether a person has closed it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Open: waiting for a person.
    #[default]
    Open,
    /// Closed by a person.
    Closed,
}
ipc::wire_enum!(Status {
    0 => Open,
    1 => Closed,
});

/// Whose it is: the host's, or a partition's.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    /// The partition's ID; 0 for the host itself.
    pub partition: u32,
    /// The partition's name; empty for the host.
    #[serde(default)]
    pub name: String,
}
ipc::wire_struct!(Source { partition, name });

impl Source {
    /// The host itself.
    #[must_use]
    pub fn host() -> Self {
        Self::default()
    }

    /// `host`, or `partition PROD`.
    #[must_use]
    pub fn words(&self) -> String {
        if self.partition == 0 {
            "host".into()
        } else {
            format!("partition {}", self.name)
        }
    }
}

/// A reference code as it was posted: its place in the host's history.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Posted {
    /// Its place in the history, from 1; 0 when it is not in it.
    pub seq: u64,
    /// The eight characters.
    pub code: String,
    /// Words 2 to 9 where the code has them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<String>,
    /// Its text as posted.
    #[serde(default)]
    pub text: String,
    /// When, Unix milliseconds.
    pub time_ms: u64,
}
ipc::wire_struct!(Posted {
    seq,
    code,
    words,
    text,
    time_ms
});

/// A line of the analysis log.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// When, seconds since the epoch.
    pub time: u64,
    /// What happened.
    pub text: String,
}
ipc::wire_struct!(Line { time, text });

/// What a list shows of a problem.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Its number: ascending, never reused.
    pub number: u64,
    /// Open or closed.
    pub status: Status,
    /// When it was first seen, seconds since the epoch.
    pub first_seen: u64,
    /// When it was last seen.
    pub last_seen: u64,
    /// How many times it was seen.
    pub count: u32,
    /// The code as last posted for it.
    pub refcode: Posted,
    /// Whose it is.
    pub source: Source,
    /// One line: the message's first-level text with its values.
    pub description: String,
    /// The message's identifier (`QVS5xxx`), whose second-level text
    /// says what to do.
    pub msgid: String,
    /// Whether the condition still holds.
    pub holds: bool,
    /// Who closed it; empty while open.
    #[serde(default)]
    pub closed_by: String,
    /// When it was closed; 0 while open.
    #[serde(default)]
    pub closed_at: u64,
}
ipc::wire_struct!(Summary {
    number,
    status,
    first_seen,
    last_seen,
    count,
    refcode,
    source,
    description,
    msgid,
    holds,
    closed_by,
    closed_at
});

/// A problem whole.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Problem {
    /// What a list shows.
    pub summary: Summary,
    /// What it is about: each disk unit, pool, virtual disk, interface,
    /// switch, partition or slot.
    #[serde(default)]
    pub resources: Vec<About>,
    /// The analysis log, oldest first.
    #[serde(default)]
    pub log: Vec<Line>,
}
ipc::wire_struct!(Problem {
    summary,
    resources,
    log
});

/// Which problems a list wants: each one set must match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Open or closed.
    pub status: Option<Status>,
    /// A partition's (its ID; 0 for the host's own).
    pub partition: Option<u32>,
    /// Last seen at or after, seconds since the epoch.
    pub since: Option<u64>,
}
ipc::wire_struct!(Filter {
    status,
    partition,
    since
});

impl Filter {
    /// Whether `s` is one it wants.
    #[must_use]
    pub fn wants(&self, s: &Summary) -> bool {
        self.status.is_none_or(|st| st == s.status)
            && self.partition.is_none_or(|p| p == s.source.partition)
            && self.since.is_none_or(|t| s.last_seen >= t)
    }
}

/// A page of a list, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    /// The problems.
    pub problems: Vec<Summary>,
    /// The next page's `before`; none when this is the last.
    pub more: Option<u64>,
}
ipc::wire_struct!(Page { problems, more });

/// What happened to a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// It was opened.
    Opened,
    /// Its condition was seen again.
    Seen,
    /// Its condition no longer holds; it stays open.
    Cleared,
    /// A person closed it.
    Closed,
    /// A person added a note to its analysis log.
    Noted,
}
ipc::wire_enum!(Notice {
    0 => Opened,
    1 => Seen,
    2 => Cleared,
    3 => Closed,
    4 => Noted,
});

/// A person's note in an analysis log: its line, and who wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Note {
    /// The line as written.
    pub line: Line,
    /// Its author, as the request named them.
    pub by: String,
}
ipc::wire_struct!(Note { line, by });

/// An event of the problem service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Noticed {
    /// What happened.
    pub notice: Notice,
    /// The problem as it is now.
    pub problem: Summary,
    /// When, seconds since the epoch.
    pub time: u64,
    /// The note, for [`Notice::Noted`].
    pub note: Option<Note>,
}
ipc::wire_struct!(Noticed {
    notice,
    problem,
    time,
    note
});

/// How grave a problem is: its condition's, by the product's rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    /// A person should look; the host carries on.
    #[default]
    Attention,
    /// The host parked, or a partition was held, for it.
    Terminal,
}
ipc::wire_enum!(Severity {
    0 => Attention,
    1 => Terminal,
});

impl Severity {
    /// As a table says it.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Severity::Attention => "attention",
            Severity::Terminal => "terminal",
        }
    }
}

/// A problem's summary with its severity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rated {
    /// How grave it is.
    pub severity: Severity,
    /// What a list shows.
    pub summary: Summary,
}
ipc::wire_struct!(Rated { severity, summary });

/// A page of a list with each problem's severity, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RatedPage {
    /// The problems.
    pub problems: Vec<Rated>,
    /// The next page's `before`; none when this is the last.
    pub more: Option<u64>,
}
ipc::wire_struct!(RatedPage { problems, more });

/// A problem whole with its severity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RatedProblem {
    /// How grave it is.
    pub severity: Severity,
    /// The problem.
    pub problem: Problem,
}
ipc::wire_struct!(RatedProblem { severity, problem });

/// How a resource is said to a person: `disk unit DD003`, `virtual disk
/// VD03 in pool POOL1`.
#[must_use]
pub fn said(a: &About) -> String {
    use supervise::control::about as kind;
    let what = match a.kind.as_str() {
        kind::DISK_UNIT => "disk unit",
        kind::STORAGE_POOL => "storage pool",
        kind::VIRTUAL_DISK => "virtual disk",
        kind::NETWORK_INTERFACE => "network interface",
        kind::VIRTUAL_SWITCH => "virtual switch",
        kind::PARTITION => "partition",
        kind::CODE_SLOT => "code slot",
        kind::SERVICE => "service",
        _ => "",
    };
    let mut out = if what.is_empty() {
        a.name.clone()
    } else {
        format!("{what} {}", a.name)
    };
    if !a.location.is_empty() {
        out = format!("{out} at {}", a.location);
    }
    if !a.serial.is_empty() {
        out = format!("{out}, serial {}", a.serial);
    }
    out
}
