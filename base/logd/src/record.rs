// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One log record. The sender stamps the time; the log service adds
//! the boot ID when it writes the line.

use appliance::time::now_nanos;

/// The longest message kept; a datagram carries one record.
pub const MESSAGE_MAX: usize = 8 * 1024;

/// How much a record matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// What happened.
    Info,
    /// Something is wrong; the host continues.
    Warning,
    /// Something failed.
    Error,
}
ipc::wire_enum!(Severity {
    0 => Info,
    1 => Warning,
    2 => Error,
});

impl Severity {
    /// The word in the log.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// One record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// When, nanoseconds since the epoch.
    pub time: i64,
    /// Who: a service's name, or PID 1's.
    pub source: String,
    /// How much it matters.
    pub severity: Severity,
    /// What.
    pub message: String,
    /// The reference code, when the record is one being posted: it
    /// goes into the history as well.
    pub code: Option<String>,
}
ipc::wire_struct!(Record {
    time,
    source,
    severity,
    message,
    code
});

impl Record {
    /// A record made now.
    pub fn new(source: &str, severity: Severity, message: &str) -> Self {
        let mut end = message.len().min(MESSAGE_MAX);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            time: now_nanos(),
            source: source.to_string(),
            severity,
            message: message[..end].to_string(),
            code: None,
        }
    }

    /// A reference code being posted now.
    pub fn code(source: &str, severity: Severity, code: &str, text: &str) -> Self {
        Self {
            code: Some(code.to_string()),
            ..Self::new(source, severity, text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_round_trips_and_is_bounded() {
        let r = Record::code("init", Severity::Warning, "A0000001", "attention");
        assert_eq!(ipc::decode::<Record>(&ipc::encode(&r)).unwrap(), r);
        let long = "é".repeat(MESSAGE_MAX);
        let r = Record::new("x", Severity::Info, &long);
        assert!(r.message.len() <= MESSAGE_MAX);
        assert!(r.message.chars().all(|c| c == 'é'));
        assert_eq!(Severity::Error.name(), "error");
    }
}
