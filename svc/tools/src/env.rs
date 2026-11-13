// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the panels of a session ask. Nothing here edits a state file:
//! the network service applies and keeps the network and sets the
//! clock, PID 1 keeps the host's own settings and does the power, the
//! storage service has the disks. The service tools users are this
//! service's own, kept through the users' store. The logs are read.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use appliance::time::{Zone, now_secs, parse_rfc3339};
use netd::control::{Calls as _, Client as NetClient, TRIAL_KEPT, TRIAL_RESTORED, TRIAL_TRYING};
use panels::Named;
use panels::forms::{LinkChoice, TimeChoice};
use panels::tools::{Host, LogLine, Trial};
use qsvsstgd::{Calls as _, Client as StorageClient};
use state::{Host as HostFile, Inventory, Network, Store};
use supervise::control::{Calls as _, Client, HostStatus};
use svcusers::FileStore;
use users::Users;

/// The lines of a log a display reads: the newest.
const LOG_LINES: usize = 2000;

/// The service tools users, shared by the sessions.
pub type SharedUsers = Arc<Mutex<Users<FileStore>>>;

/// The users of this host.
pub fn open_users() -> SharedUsers {
    let path = Path::new(release::DATA_DIR).join("state").join(users::FILE);
    let users = Users::open(&path).unwrap_or_else(|e| {
        eprintln!("{e}");
        // An unreadable file is no users: nobody signs on.
        Users::open(PathBuf::from("/run/qsvstools.users")).expect("an empty store opens")
    });
    Arc::new(Mutex::new(users))
}

fn wire(e: impl std::fmt::Display, who: &str) -> String {
    format!("{who} does not answer: {e}.")
}

/// One session's environment.
pub struct Env {
    /// Where the session is: `display` or `serial port`.
    pub place: &'static str,
    /// Who is signed on.
    pub user: String,
    users: SharedUsers,
    control: Client,
    net: NetClient,
    storage: StorageClient,
    store: Store,
    /// The host name, as last seen in PID 1's status.
    name: String,
}

impl Env {
    /// The environment of a session at `place`.
    pub fn new(place: &'static str, users: SharedUsers) -> Self {
        let appliance = release::appliance(|_| {}, |_, _| {});
        Self {
            place,
            user: String::new(),
            users,
            control: Client::new(appliance.control_socket()),
            net: NetClient::new(appliance.net_socket()),
            storage: StorageClient::default(),
            store: Store::at(appliance.state_dir()),
            name: release::NAME.into(),
        }
    }

    /// PID 1's status.
    pub fn status(&mut self) -> HostStatus {
        let status = self.control.status().unwrap_or_default();
        if let Some(host) = status.fact(release::fact::HOST).filter(|h| !h.is_empty()) {
            self.name = host.to_string();
        }
        status
    }

    /// The storage service's status; `None` when it does not answer.
    pub fn storage(&mut self) -> Option<qsvsstgd::Status> {
        self.storage.status().ok()
    }

    /// Start mirrored protection with `disk`.
    pub fn start_protection(&mut self, disk: &str) -> Result<(), String> {
        self.storage
            .start_protection(disk.to_string())
            .map_err(|e| wire(e, "The storage service"))?
            .map_err(|e| sentence(&e))
    }

    /// Put `disk` in the place of unit `number`.
    pub fn replace(&mut self, number: u32, disk: &str) -> Result<(), String> {
        self.storage
            .replace(number, disk.to_string())
            .map_err(|e| wire(e, "The storage service"))?
            .map_err(|e| sentence(&e))
    }

    /// The display zone.
    pub fn zone(&self) -> Zone {
        self.host_file().zone()
    }

    fn host_file(&self) -> HostFile {
        self.store
            .load::<HostFile>()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Tell PID 1 a fact.
    pub fn note(&mut self, key: &str, value: &str) {
        let _ = self.control.note(key.to_string(), value.to_string());
    }
}

/// A record's time in the display zone `zone`, to the minute.
fn stamp(zone: Zone, value: &serde_json::Value) -> String {
    value["time"]
        .as_str()
        .and_then(parse_rfc3339)
        .map_or_else(String::new, |secs| zone.display(secs)[..16].to_string())
}

/// The lines of the log files named, newest first, at most
/// [`LOG_LINES`]: each parsed line through `line`.
fn log(files: &[&str], line: impl Fn(&serde_json::Value) -> Option<LogLine>) -> Vec<LogLine> {
    let dir = Path::new(release::DATA_DIR).join("log");
    let mut out = Vec::new();
    // The files are named newest first.
    for file in files {
        let text = std::fs::read_to_string(dir.join(file)).unwrap_or_default();
        for raw in text.lines().rev() {
            if out.len() >= LOG_LINES {
                return out;
            }
            if let Some(parsed) = serde_json::from_str(raw).ok().and_then(|v| line(&v)) {
                out.push(parsed);
            }
        }
    }
    out
}

/// The machine's inventory, read now.
pub fn inventory() -> Inventory {
    hostinv::read(&hostinv::Roots::default(), None)
}

/// A reason as a message line words it.
fn sentence(reason: &str) -> String {
    let mut chars = reason.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>());
    let rest = chars.as_str();
    let stop = if rest.ends_with('.') { "" } else { "." };
    format!("{}{rest}{stop}", first.unwrap_or_default())
}

impl Named for Env {
    fn system(&self) -> String {
        self.name.clone()
    }
}

impl Host for Env {
    fn now(&self) -> i64 {
        now_secs()
    }

    fn sign_on(&mut self, id: &str, password: &str) -> Result<String, String> {
        let now = u64::try_from(now_secs()).unwrap_or(0);
        let checked = self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sign_on(id, password, now);
        // The attempt is logged; the password never is.
        let asked = svcusers::normalize_name(id);
        match checked {
            Ok(id) => {
                println!("service tools user {id} signed on at the {}", self.place);
                Ok(id)
            }
            Err(refused) => {
                let why = match refused {
                    users::Refused::NotCorrect => "user ID or password not correct".to_string(),
                    users::Refused::Disabled(left) => format!("user ID disabled for {left} s"),
                };
                eprintln!(
                    "sign-on refused at the {} for service tools user {asked:?}: {why}",
                    self.place
                );
                Err(refused.to_string())
            }
        }
    }

    fn user_ids(&mut self) -> Vec<String> {
        self.users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ids()
    }

    fn add_user(&mut self, id: &str, password: &str) -> Result<(), String> {
        let now = u64::try_from(now_secs()).unwrap_or(0);
        let done = self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .add(id, password, now);
        if done.is_ok() {
            println!("service tools user {id} added by {}", self.user);
        }
        done
    }

    fn set_password(&mut self, id: &str, password: &str) -> Result<(), String> {
        let now = u64::try_from(now_secs()).unwrap_or(0);
        let done = self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_password(id, password, now);
        if done.is_ok() {
            println!(
                "password of service tools user {id} changed by {}",
                self.user
            );
        }
        done
    }

    fn remove_user(&mut self, id: &str) -> Result<(), String> {
        let done = self
            .users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        if done.is_ok() {
            println!("service tools user {id} removed by {}", self.user);
        }
        done
    }

    fn links(&mut self) -> Vec<LinkChoice> {
        inventory()
            .interfaces
            .into_iter()
            .map(|i| LinkChoice {
                name: i.name,
                mac: i.mac,
                location: i.location,
                carrier: i.carrier,
            })
            .collect()
    }

    fn settings(&mut self) -> (HostFile, Network) {
        let mut host = self.host_file();
        self.status();
        host.name = Some(self.name.clone());
        let network = self
            .store
            .load::<Network>()
            .ok()
            .flatten()
            .unwrap_or_default();
        (host, network)
    }

    fn set_name(&mut self, name: &str) -> Result<(), String> {
        let host = self.host_file();
        self.control
            .set_host(name.to_string(), host.time_zone, host.ntp_servers)
            .map_err(|e| wire(e, "PID 1"))?
            .map_err(|e| sentence(&e))?;
        self.name = name.to_string();
        println!("host name changed to {name} by {}", self.user);
        // The network service gives the name to DHCP.
        let _ = self.net.reload();
        Ok(())
    }

    fn apply_network(&mut self, network: &Network) -> Result<(), String> {
        let text = state::store::render(network).map_err(|e| sentence(&e.to_string()))?;
        self.net
            .apply(text)
            .map_err(|e| wire(e, "The network service"))?
            .map_err(|e| sentence(&e))?;
        println!("management network change put on trial by {}", self.user);
        Ok(())
    }

    fn trial(&mut self) -> (Trial, String) {
        match self.net.trial() {
            Ok(t) => {
                let trial = match t.state.as_str() {
                    TRIAL_TRYING => Trial::Trying(t.seconds_left),
                    TRIAL_KEPT => Trial::Kept,
                    TRIAL_RESTORED => Trial::Restored(t.reason),
                    _ => Trial::Unknown,
                };
                (trial, t.line)
            }
            Err(e) => (Trial::Unknown, wire(e, "The network service")),
        }
    }

    fn confirm_network(&mut self) -> Result<(), String> {
        self.net
            .confirm()
            .map_err(|e| wire(e, "The network service"))?
            .map_err(|e| sentence(&e))
    }

    fn set_time(&mut self, choice: &TimeChoice) -> Result<(), String> {
        self.status();
        self.control
            .set_host(
                self.name.clone(),
                choice.zone.clone(),
                choice.servers.clone(),
            )
            .map_err(|e| wire(e, "PID 1"))?
            .map_err(|e| sentence(&e))?;
        // The network service keeps the time: it reads the servers again.
        let _ = self.net.reload();
        if let Some(secs) = choice.clock {
            self.net
                .set_clock(secs)
                .map_err(|e| wire(e, "The network service"))?
                .map_err(|e| sentence(&e))?;
        }
        println!("date and time settings changed by {}", self.user);
        Ok(())
    }

    fn history(&mut self) -> Vec<LogLine> {
        let zone = self.zone();
        log(&["refcodes.ndjson"], |v| {
            let code = v["code"].as_str()?;
            Some(LogLine {
                time: stamp(zone, v),
                severity: match code.as_bytes().first() {
                    Some(b'A') => 1,
                    Some(b'B') => 2,
                    _ => 0,
                },
                text: format!("{code} {}", v["text"].as_str().unwrap_or("")),
            })
        })
    }

    fn system_log(&mut self) -> Vec<LogLine> {
        let zone = self.zone();
        log(&["system.ndjson", "system.1.ndjson"], |v| {
            Some(LogLine {
                time: stamp(zone, v),
                severity: match v["severity"].as_str()? {
                    "warning" => 1,
                    "error" => 2,
                    _ => 0,
                },
                text: format!(
                    "{}: {}",
                    v["source"].as_str().unwrap_or("?"),
                    v["message"].as_str().unwrap_or("")
                ),
            })
        })
    }

    fn power(&mut self, restart: bool) -> Result<(), String> {
        println!(
            "{} asked for by {} at the {}",
            if restart { "restart" } else { "power off" },
            self.user,
            self.place
        );
        let asked = if restart {
            self.control.restart()
        } else {
            self.control.power_off()
        };
        asked.map_err(|e| wire(e, "PID 1"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_read_as_sentences() {
        assert_eq!(sentence("disk vdb is in use"), "Disk vdb is in use.");
        assert_eq!(sentence("Already one."), "Already one.");
        assert_eq!(wire("refused", "PID 1"), "PID 1 does not answer: refused.");
    }
}
