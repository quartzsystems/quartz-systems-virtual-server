// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the server reads of the host: the inventory, the host's name
//! and zone (state files, read only), the reference code history (the
//! log service's file), and PID 1's state and power, over its socket.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use qsi::PeerKind;
use qsi::base::{Identity, Time};
use qsisess::Note;
use supervise::control::{Calls as _, Client, HostState};

use crate::families::base::Posted;
use crate::families::{Facts, down};

/// Where the host keeps what the server reads.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The state files.
    pub state: PathBuf,
    /// The log service's files.
    pub log: PathBuf,
    /// PID 1's control socket.
    pub control: PathBuf,
    /// The host's own dumps.
    pub dumps: PathBuf,
}

/// The facts, from the files and PID 1.
#[derive(Clone, Debug)]
pub struct HostFacts {
    /// Where they are.
    pub paths: Paths,
    /// The release that runs, its build.
    pub release: String,
    /// The build's identifier.
    pub build: String,
}

fn unread(what: &str, e: &dyn std::fmt::Display) -> Vec<Note> {
    vec![Note {
        msgid: qsi::host::host::msgid::SERVICE_DOWN.into(),
        text: format!("The host's {what} could not be read."),
        help: e.to_string(),
    }]
}

/// The reference code history's lines, oldest first: each a JSON
/// object with `time`, `code` and `text`. A line that does not read is
/// passed over.
pub fn history_of(lines: impl Iterator<Item = String>) -> Vec<Posted> {
    #[derive(serde::Deserialize)]
    struct Line {
        time: String,
        code: String,
        #[serde(default)]
        text: String,
    }
    lines
        .filter_map(|l| serde_json::from_str::<Line>(&l).ok())
        .map(|l| Posted {
            time_ms: appliance::time::parse_rfc3339(&l.time)
                .and_then(|s| u64::try_from(s).ok())
                .map_or(0, |s| s.saturating_mul(1000)),
            code: l.code,
            text: l.text,
        })
        .collect()
}

impl HostFacts {
    fn host(&self) -> state::Host {
        state::Store::at(&self.paths.state)
            .load::<state::Host>()
            .ok()
            .flatten()
            .unwrap_or_default()
    }
}

impl Facts for HostFacts {
    fn inventory(&mut self) -> Result<state::Inventory, Vec<Note>> {
        state::Store::at(&self.paths.state)
            .load::<state::Inventory>()
            .map_err(|e| unread("inventory", &e))?
            .ok_or_else(|| unread("inventory", &"it is not there"))
    }

    fn identity(&mut self) -> Result<Identity, Vec<Note>> {
        let inventory = self.inventory().unwrap_or_default();
        Ok(Identity {
            kind: PeerKind::Host,
            name: self.host().name.unwrap_or_default(),
            release: self.release.clone(),
            serial: inventory.system.serial,
            model: inventory.system.product,
            build: self.build.clone(),
        })
    }

    fn state(&mut self) -> Result<HostState, Vec<Note>> {
        Client::new(&self.paths.control)
            .state()
            .map_err(|e| down("PID 1", &e))
    }

    fn power(
        &mut self,
        restart: bool,
        immediate: bool,
        delay: u32,
        by: &str,
    ) -> Result<(), Vec<Note>> {
        let by = format!("a management console, user {by:?}");
        Client::new(&self.paths.control)
            .power_down(restart, immediate, delay, by)
            .map_err(|e| down("PID 1", &e))?
            .map_err(|refused| {
                vec![Note {
                    msgid: qsi::msgid::BAD.into(),
                    text: refused,
                    help: String::new(),
                }]
            })
    }

    fn history(&mut self) -> Result<Vec<Posted>, Vec<Note>> {
        let path = self.paths.log.join(logd::files::HISTORY);
        let file = std::fs::File::open(&path).map_err(|e| unread("reference code history", &e))?;
        Ok(history_of(
            BufReader::new(file).lines().map_while(Result::ok),
        ))
    }

    fn host_dumps(&mut self) -> Vec<hostdump::Info> {
        hostdump::store::at(&self.paths.dumps).list()
    }

    fn take_host_dump(&mut self, by: &str) -> Result<String, Vec<Note>> {
        Client::new(&self.paths.control)
            .take_dump(by.to_string())
            .map_err(|e| down("PID 1", &e))?
            .map_err(|why| vec![crate::families::base::taking(&why)])
    }

    fn delete_host_dump(&mut self, name: &str) -> Result<(), Vec<Note>> {
        let store = hostdump::store::at(&self.paths.dumps);
        if store.path(name).is_none_or(|p| !p.is_file()) {
            return Err(vec![crate::families::base::no_dump(name)]);
        }
        store.delete(name).map_err(|why| {
            vec![Note {
                msgid: qsi::base::msgid::NO_DUMP.into(),
                text: why,
                help: String::new(),
            }]
        })?;
        let code = &refcode::HOST_DUMP_REMOVED;
        let _ = Client::new(&self.paths.control).post(
            code.code.to_string(),
            code.text_with(&format!("{name}, by a management console")),
        );
        Ok(())
    }

    fn time(&mut self) -> Time {
        let zone: appliance::time::Zone = self.host().time_zone.parse().unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
        Time {
            unix_ns: now,
            offset_s: zone.minutes * 60,
            zone: zone.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_reads_each_line_it_can() {
        let lines = [
            r#"{"time":"2026-11-12T10:22:03.123456789Z","boot":"b","code":"C2001005","text":"Host running"}"#,
            "not json",
            r#"{"time":"2026-11-12T10:22:04Z","boot":"b","code":"A2001040"}"#,
        ];
        let h = history_of(lines.iter().map(|l| (*l).to_string()));
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].code, "C2001005");
        assert_eq!(h[0].time_ms % 1000, 0);
        assert_eq!(h[1].time_ms - h[0].time_ms, 1000);
    }
}
