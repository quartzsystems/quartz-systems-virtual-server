// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a soak found: each IPL and each fault as a JSON line in
//! `rounds.jsonl`, and the totals with their rates.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How one IPL of one partition went.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Round {
    /// The partition.
    pub partition: String,
    /// Its kind: `qsvs` or `qs400`.
    pub kind: String,
    /// The IPL's number for this partition.
    pub ipl: u64,
    /// Seconds from activation until the system was up, when it was.
    pub up_secs: Option<f64>,
    /// Seconds the IPL lasted in all.
    pub secs: f64,
    /// The lines that named a fault.
    pub faults: Vec<String>,
    /// Whether the partition was held.
    pub held: bool,
    /// The dump copied out, when one was taken.
    pub dump: Option<String>,
    /// The state and code it ended with, when not by the soak's shut down.
    pub end: Option<String>,
}

/// The soak's report directory and its counts.
pub struct Report {
    dir: PathBuf,
    /// IPLs begun.
    pub ipls: u64,
    /// IPLs that reached the system.
    pub up: u64,
    /// IPLs with a fault.
    pub faulted: u64,
    /// IPLs that never came up.
    pub failed: u64,
}

impl Report {
    /// A report in `dir`, made when it is not there.
    pub fn new(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir.join("dumps")).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            ipls: 0,
            up: 0,
            faulted: 0,
            failed: 0,
        })
    }

    /// Where a dump of `name`'s IPL `ipl` goes.
    pub fn dump_path(&self, name: &str, ipl: u64) -> PathBuf {
        self.dir.join("dumps").join(format!("{name}-{ipl}.core"))
    }

    /// Where a file of the report goes.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Count `round` and write it down.
    pub fn add(&mut self, round: &Round) {
        self.ipls += 1;
        if round.up_secs.is_some() {
            self.up += 1;
        } else {
            self.failed += 1;
        }
        if !round.faults.is_empty() || round.held {
            self.faulted += 1;
        }
        if let Ok(line) = serde_json::to_string(round)
            && let Ok(mut f) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join("rounds.jsonl"))
        {
            let _ = writeln!(f, "{line}");
        }
    }

    /// The totals after `elapsed`, with the rates.
    pub fn summary(&self, elapsed: Duration, settings: &str) -> String {
        let hours = elapsed.as_secs_f64() / 3600.0;
        #[allow(clippy::cast_precision_loss)] // counts far below 2^52
        let per_thousand = if self.ipls == 0 {
            0.0
        } else {
            self.faulted as f64 * 1000.0 / self.ipls as f64
        };
        #[allow(clippy::cast_precision_loss)]
        let per_hour = if hours > 0.0 {
            self.faulted as f64 / hours
        } else {
            0.0
        };
        let text = format!(
            "{settings}: {} IPL(s) in {hours:.2} h, {} up, {} never up, {} with a fault: \
             {per_thousand:.1} per 1000 IPLs, {per_hour:.2} per hour",
            self.ipls, self.up, self.failed, self.faulted
        );
        let _ = fs::write(self.dir.join("summary.txt"), format!("{text}\n"));
        text
    }
}
