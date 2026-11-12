// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Posting a code: one line on the console, `REF <code> <text>`, and
//! one record for the log service, which keeps the history. PID 1
//! posts; a service posts through PID 1's control socket ([`remote`]).

#[cfg(unix)]
pub mod remote;

use logd::{Record, Severity};

use crate::{Family, RefCode, WORD};

/// Who posts, in the log.
pub const SOURCE: &str = "init";

/// How much a family's codes matter in the log.
pub fn severity(family: Family) -> Severity {
    match family {
        Family::Progress | Family::PowerDown => Severity::Info,
        Family::Attention => Severity::Warning,
        Family::Fatal => Severity::Error,
    }
}

/// One posting: what goes on the console and what goes to the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posting {
    /// The console line.
    pub line: String,
    /// The record; the log service puts it in the history too.
    pub record: Record,
}

/// Post `code`, with `detail` added to its text when there is some.
pub fn posting(code: &RefCode, detail: &str) -> Posting {
    let text = code.text_with(detail);
    Posting {
        line: format!("{WORD} {} {text}", code.code),
        record: Record::code(SOURCE, severity(code.family), code.code, &text),
    }
}

/// A posting a service sent as a code and its whole text. `None` when
/// the code is not in the table: nothing else is ever posted.
pub fn posting_of(code: &str, text: &str) -> Option<Posting> {
    let known = crate::find(code)?;
    Some(Posting {
        line: format!("{WORD} {} {text}", known.code),
        record: Record::code(SOURCE, severity(known.family), known.code, text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posting_is_the_line_and_the_history_record() {
        let p = posting(&crate::SERVICE_STOPPED, "netd");
        assert_eq!(p.line, "REF A2001080 Service stopped: netd");
        assert_eq!(p.record.code.as_deref(), Some("A2001080"));
        assert_eq!(p.record.message, "Service stopped: netd");
        assert_eq!(p.record.severity, Severity::Warning);
        assert_eq!(p.record.source, "init");
        assert_eq!(
            posting(&crate::BASE_READY, "").line,
            "REF C2001090 Base ready"
        );
        assert_eq!(posting(&crate::PANIC, "").record.severity, Severity::Error);
    }

    #[test]
    fn a_service_posts_only_what_the_table_has() {
        let p = posting_of("C2002010", "Management interface up: eth0").unwrap();
        assert_eq!(p.line, "REF C2002010 Management interface up: eth0");
        assert_eq!(p.record.severity, Severity::Info);
        assert_eq!(posting_of("C1000000", "another system's"), None);
    }
}
