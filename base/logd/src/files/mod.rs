// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What is kept under the log directory:
//!
//! - `system.ndjson`: every record, one JSON object a line, with
//!   `system.1.ndjson` (the newest) and so on behind it.
//! - `refcodes.ndjson`: every reference code posted, in order,
//!   append-only and never rotated.
//! - `kernel/<boot id>.log`: the kernel log of each boot.

pub mod kernel;
mod sink;

use std::fs;
use std::path::{Path, PathBuf};

use appliance::time::rfc3339;
use serde::Serialize;

use crate::record::{Record, Severity};
pub use sink::Sink;

/// The system log's name.
pub const SYSTEM: &str = "system";
/// The reference code history's file.
pub const HISTORY: &str = "refcodes.ndjson";
/// The file names' ending.
pub const EXT: &str = "ndjson";

/// How much is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes the system log grows to before it is rotated.
    pub file_bytes: u64,
    /// Bytes all system log files together may take.
    pub total_bytes: u64,
    /// Bytes of one boot's kernel log.
    pub kernel_bytes: u64,
    /// Boots whose kernel log is kept.
    pub kernel_boots: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 4 << 20,
            total_bytes: 64 << 20,
            kernel_bytes: 4 << 20,
            kernel_boots: 8,
        }
    }
}

/// A line of the system log.
#[derive(Serialize)]
struct SystemLine<'a> {
    time: String,
    boot: &'a str,
    source: &'a str,
    severity: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
}

/// A line of the reference code history.
#[derive(Serialize)]
struct HistoryLine<'a> {
    time: String,
    boot: &'a str,
    code: &'a str,
    text: &'a str,
}

fn json(line: &impl Serialize) -> String {
    serde_json::to_string(line).unwrap_or_else(|_| "{}".into())
}

/// The system log line of a record.
pub fn system_line(record: &Record, boot: &str) -> String {
    json(&SystemLine {
        time: rfc3339(record.time),
        boot,
        source: &record.source,
        severity: record.severity.name(),
        message: &record.message,
        code: record.code.as_deref(),
    })
}

/// The history line of a record that posts a code.
pub fn history_line(record: &Record, boot: &str) -> Option<String> {
    let code = record.code.as_deref()?;
    Some(json(&HistoryLine {
        time: rfc3339(record.time),
        boot,
        code,
        text: &record.message,
    }))
}

/// `<name>.ndjson`, or `<name>.<n>.ndjson` for an older one.
pub fn rotated(dir: &Path, name: &str, n: usize) -> PathBuf {
    if n == 0 {
        dir.join(format!("{name}.{EXT}"))
    } else {
        dir.join(format!("{name}.{n}.{EXT}"))
    }
}

/// The log directory's files.
#[derive(Debug)]
pub struct Logs {
    boot: String,
    system: Sink,
    history: Sink,
    dirty: bool,
}

impl Logs {
    /// The logs in `dir` for the boot `boot`.
    pub fn open(dir: &Path, boot: &str, limits: Limits) -> Self {
        Self {
            boot: boot.to_string(),
            system: Sink::rotating(dir, SYSTEM, limits.file_bytes, limits.total_bytes),
            history: Sink::append_only(dir.join(HISTORY)),
            dirty: false,
        }
    }

    /// Write one record: to the system log, and to the history when it
    /// posts a code. A code and an error reach the disk at once.
    pub fn write(&mut self, record: &Record) {
        self.system.line(&system_line(record, &self.boot));
        self.dirty = true;
        if let Some(line) = history_line(record, &self.boot) {
            self.history.line(&line);
            self.history.sync();
        }
        if record.code.is_some() || record.severity == Severity::Error {
            self.sync();
        }
    }

    /// Bring the system log to the disk, if anything was written.
    pub fn sync(&mut self) {
        if self.dirty {
            self.system.sync();
            self.dirty = false;
        }
    }

    /// Lines held in memory because the directory cannot be written.
    pub fn held(&self) -> usize {
        self.system.held() + self.history.held()
    }
}

/// The boot ID the kernel made for this boot.
pub fn boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|t| t.trim().to_string())
        .ok()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("logd-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn record(time: i64, message: &str, code: Option<&str>) -> Record {
        Record {
            time,
            source: "init".into(),
            severity: Severity::Info,
            message: message.into(),
            code: code.map(str::to_string),
        }
    }

    #[test]
    fn records_are_json_lines_and_codes_go_to_the_history_too() {
        let dir = scratch("lines");
        let mut logs = Logs::open(&dir, "boot-1", Limits::default());
        logs.write(&record(1_000_000_000, "said \"hi\"", None));
        logs.write(&record(2_000_000_000, "Host IPL started", Some("C0000001")));
        logs.sync();
        let system = fs::read_to_string(rotated(&dir, SYSTEM, 0)).unwrap();
        let lines: Vec<serde_json::Value> = system
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["time"], "1970-01-01T00:00:01.000000000Z");
        assert_eq!(lines[0]["boot"], "boot-1");
        assert_eq!(lines[0]["source"], "init");
        assert_eq!(lines[0]["severity"], "info");
        assert_eq!(lines[0]["message"], "said \"hi\"");
        assert!(lines[0].get("code").is_none());
        assert_eq!(lines[1]["code"], "C0000001");
        let history = fs::read_to_string(dir.join(HISTORY)).unwrap();
        let h: serde_json::Value = serde_json::from_str(history.trim_end()).unwrap();
        assert_eq!(history.lines().count(), 1);
        assert_eq!(
            (h["code"].as_str(), h["text"].as_str(), h["boot"].as_str()),
            (Some("C0000001"), Some("Host IPL started"), Some("boot-1"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_history_is_appended_to_across_boots() {
        let dir = scratch("history");
        for boot in ["boot-1", "boot-2"] {
            let mut logs = Logs::open(&dir, boot, Limits::default());
            logs.write(&record(0, "a", Some("C0000001")));
            logs.write(&record(0, "b", Some("C0000002")));
        }
        let history = fs::read_to_string(dir.join(HISTORY)).unwrap();
        let boots: Vec<String> = history
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["boot"].to_string())
            .collect();
        assert_eq!(
            boots,
            ["\"boot-1\"", "\"boot-1\"", "\"boot-2\"", "\"boot-2\""]
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
