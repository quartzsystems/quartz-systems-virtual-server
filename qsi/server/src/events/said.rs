// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the log last said of each thing, kept beside it on disk: each
//! partition's state and last code, each pool, disk, image, volume and
//! switch, and the host's place in its history. A server that takes
//! up its subscriptions again compares the services with it and says
//! what changed while it was down (`docs/QSISERVER.md`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use qsi::any::AnyEvent;
use qsi::host::{partitions, refcodes, vres};

use super::store::{hex, unhex};
use super::translate::{self, Look};
use crate::families::{convert, vres as v};

/// The file, under the server's state directory.
pub const FILE: &str = "qsi-said.txt";

/// What the log last said.
#[derive(Debug, Default)]
pub struct Said {
    map: BTreeMap<String, String>,
    path: Option<PathBuf>,
    dirty: bool,
}

fn encoded<T: ipc::Wire>(value: &T) -> String {
    hex(&ipc::encode(value))
}

fn decoded<T: ipc::Wire>(text: &str) -> Option<T> {
    ipc::decode(&unhex(text)?).ok()
}

fn part(id: u32, what: &str) -> String {
    format!("part.{id}.{what}")
}

/// A code said: when, and each code said at that time.
fn code_of(text: &str) -> (u64, BTreeSet<String>) {
    let mut f = text.split(' ');
    let time = f.next().and_then(|t| t.parse().ok()).unwrap_or(0);
    (time, f.map(str::to_string).collect())
}

impl Said {
    /// What the file in `dir` holds; nothing when it is not there.
    #[must_use]
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let mut map = BTreeMap::new();
        if let Ok(f) = File::open(&path) {
            for l in BufReader::new(f).lines().map_while(Result::ok) {
                if let Some((k, v)) = l.split_once('\t') {
                    map.insert(k.to_string(), v.to_string());
                }
            }
        }
        Self {
            map,
            path: Some(path),
            dirty: false,
        }
    }

    fn set(&mut self, key: String, value: String) -> bool {
        if key.contains(['\t', '\n']) || value.contains('\n') {
            return true;
        }
        if self.map.get(&key) == Some(&value) {
            return false;
        }
        self.map.insert(key, value);
        self.dirty = true;
        true
    }

    fn remove(&mut self, key: &str) -> bool {
        let gone = self.map.remove(key).is_some();
        self.dirty |= gone;
        gone
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    /// Whether a source was ever compared: one never compared has its
    /// first look taken as what was said, and nothing pushed for it.
    #[must_use]
    pub fn knows(&self, source: &str) -> bool {
        self.map.contains_key(&format!("source.{source}"))
    }

    /// Mark a source compared.
    pub fn known(&mut self, source: &str) {
        self.set(format!("source.{source}"), "1".into());
    }

    /// Note what an event says. False when it says nothing the log has
    /// not said: a state again, a code said already, a thing as it was.
    pub fn note(&mut self, e: &AnyEvent) -> bool {
        match e {
            AnyEvent::Partitions(partitions::Event::State(s)) => {
                self.set(part(s.id, "name"), s.name.clone());
                self.set(part(s.id, "state"), encoded(&s.state))
            }
            AnyEvent::Partitions(partitions::Event::Defined(d)) => {
                if d.exists {
                    return self.set(part(d.id, "name"), d.name.clone());
                }
                let prefix = format!("part.{}.", d.id);
                let keys: Vec<String> = self
                    .map
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .cloned()
                    .collect();
                let mut gone = false;
                for k in keys {
                    gone |= self.remove(&k);
                }
                gone
            }
            AnyEvent::Refcodes(refcodes::Event::Code(c)) => {
                let key = part(c.id, "code");
                let r = &c.posted.refcode;
                let (time, mut codes) = self.get(&key).map(code_of).unwrap_or_default();
                if r.time_ms < time || (r.time_ms == time && codes.contains(&r.code)) {
                    return false;
                }
                if r.time_ms > time {
                    codes.clear();
                }
                codes.insert(r.code.clone());
                let mut value = r.time_ms.to_string();
                for c in &codes {
                    value.push(' ');
                    value.push_str(c);
                }
                self.set(key, value);
                true
            }
            AnyEvent::Base(qsi::base::Event::DumpTaken(d)) => match &d.dump {
                Some(dump) => self.set(format!("hostdump.{}", d.name), dump.bytes.to_string()),
                None => true,
            },
            AnyEvent::Base(qsi::base::Event::DumpRemoved(d)) => {
                self.remove(&format!("hostdump.{}", d.name))
            }
            AnyEvent::Base(qsi::base::Event::Refcode(r)) => {
                let seen = self.host_seen().unwrap_or(0);
                if r.seq != 0 && r.seq <= seen {
                    return false;
                }
                self.set("host.seen".into(), r.seq.to_string());
                true
            }
            AnyEvent::Vres(e) => self.note_vres(e),
            AnyEvent::Service(e) => {
                let (qsi::service::Event::Opened(n)
                | qsi::service::Event::Seen(n)
                | qsi::service::Event::Cleared(n)
                | qsi::service::Event::Closed(n)) = e;
                self.set(format!("problem.{}", n.problem.number), encoded(&n.problem))
            }
            _ => true,
        }
    }

    fn keep<T: ipc::Wire>(&mut self, key: String, value: Option<&T>) -> bool {
        match value {
            Some(v) => self.set(key, encoded(v)),
            None => self.remove(&key),
        }
    }

    fn note_vres(&mut self, e: &vres::Event) -> bool {
        match e {
            vres::Event::Pool(p) => self.keep(format!("pool.{}", p.name), p.pool.as_ref()),
            vres::Event::Vdisk(d) => {
                self.keep(format!("vdisk.{}/{}", d.pool, d.name), d.vdisk.as_ref())
            }
            vres::Event::Image(i) => self.keep(format!("image.{}", i.name), i.image.as_ref()),
            vres::Event::Volume(o) => self.keep(format!("volume.{}", o.name), o.volume.as_ref()),
            vres::Event::Switch(s) => self.keep(format!("switch.{}", s.name), s.switch.as_ref()),
            _ => true,
        }
    }

    /// The host's last history place said.
    #[must_use]
    pub fn host_seen(&self) -> Option<u64> {
        self.get("host.seen")?.parse().ok()
    }

    /// The pool service as the log last said it.
    #[must_use]
    pub fn look(&self) -> Look {
        let mut look = Look::default();
        for (k, value) in &self.map {
            if let Some(name) = k.strip_prefix("pool.") {
                if let Some(p) = decoded(value) {
                    look.pools.insert(name.to_string(), p);
                }
            } else if let Some(key) = k.strip_prefix("vdisk.") {
                if let (Some((pool, name)), Some(d)) = (key.split_once('/'), decoded(value)) {
                    look.vdisks.insert((pool.to_string(), name.to_string()), d);
                }
            } else if let Some(name) = k.strip_prefix("image.") {
                if let Some(i) = decoded(value) {
                    look.images.insert(name.to_string(), i);
                }
            } else if let Some(name) = k.strip_prefix("volume.")
                && let Some(o) = decoded(value)
            {
                look.volumes.insert(name.to_string(), o);
            }
        }
        look
    }

    /// The partitions the log last said, by ID, with their names.
    fn partitions(&self) -> BTreeMap<u32, String> {
        self.map
            .iter()
            .filter_map(|(k, v)| {
                let id = k.strip_prefix("part.")?.strip_suffix(".name")?;
                Some((id.parse().ok()?, v.clone()))
            })
            .collect()
    }

    /// The switches the log last said, by name.
    fn switches(&self) -> BTreeMap<String, String> {
        self.map
            .iter()
            .filter_map(|(k, v)| Some((k.strip_prefix("switch.")?.to_string(), v.clone())))
            .collect()
    }

    /// Write what was said, when it changed.
    pub fn sync(&mut self) {
        let Some(path) = &self.path else { return };
        if !self.dirty {
            return;
        }
        let tmp = path.with_extension("tmp");
        let written = File::create(&tmp).and_then(|mut f| {
            for (k, v) in &self.map {
                writeln!(f, "{k}\t{v}")?;
            }
            f.sync_all()
        });
        if written.is_ok() && std::fs::rename(&tmp, path).is_ok() {
            self.dirty = false;
        }
    }
}

/// What the partition manager has that the log did not say: each
/// partition made or deleted, each state, and each code of a history
/// after the last said. `history` gives a partition's, oldest first.
pub fn partitions(
    said: &Said,
    list: &[partd::Status],
    mut history: impl FnMut(u32) -> Vec<partd::Posted>,
    now: u64,
) -> Vec<AnyEvent> {
    let mut out = Vec::new();
    let before = said.partitions();
    for s in list {
        if !before.contains_key(&s.id) {
            out.push(AnyEvent::Partitions(partitions::Event::Defined(
                partitions::Defined {
                    id: s.id,
                    name: s.name.clone(),
                    exists: true,
                    time: now,
                },
            )));
        }
        let state = convert::state(s.state);
        if said.get(&part(s.id, "state")) != Some(encoded(&state).as_str()) {
            out.push(AnyEvent::Partitions(partitions::Event::State(
                partitions::StateChanged {
                    id: s.id,
                    name: s.name.clone(),
                    state,
                    time: now,
                },
            )));
        }
        let (time, codes) = said
            .get(&part(s.id, "code"))
            .map(code_of)
            .unwrap_or_default();
        for p in history(s.id) {
            let ms = p.time.saturating_mul(1000);
            if ms > time || (ms == time && !codes.contains(&p.code)) {
                out.push(translate::partd(&partd::Event::Code {
                    id: s.id,
                    name: s.name.clone(),
                    posted: p,
                }));
            }
        }
    }
    for (id, name) in before {
        if !list.iter().any(|s| s.id == id) {
            out.push(AnyEvent::Partitions(partitions::Event::Defined(
                partitions::Defined {
                    id,
                    name,
                    exists: false,
                    time: now,
                },
            )));
        }
    }
    out
}

/// What the host's dumps are that the log did not say: each written
/// or removed.
#[must_use]
pub fn host_dumps(said: &Said, list: &[hostdump::Info], now: u64) -> Vec<AnyEvent> {
    use qsi::base::{DumpRemoved, DumpTaken, Event};
    let before: BTreeSet<String> = said
        .map
        .keys()
        .filter_map(|k| k.strip_prefix("hostdump.").map(str::to_string))
        .collect();
    let mut out = Vec::new();
    for i in list {
        if !before.contains(&i.name) {
            out.push(AnyEvent::Base(Event::DumpTaken(DumpTaken {
                name: i.name.clone(),
                dump: Some(crate::families::base::system_dump(i)),
                error: None,
                time: now,
            })));
        }
    }
    for name in before {
        if !list.iter().any(|i| i.name == name) {
            out.push(AnyEvent::Base(Event::DumpRemoved(DumpRemoved {
                name,
                time: now,
            })));
        }
    }
    out
}

/// What the problem service has that the log did not say: each problem
/// opened, seen again, cleared or closed since. A problem the log never
/// said is said opened, and closed too when it is.
pub fn problems(said: &Said, list: &[problem::Summary], now: u64) -> Vec<AnyEvent> {
    use qsi::service::{Event as E, Noticed};
    let mut out = Vec::new();
    // Oldest first, as they happened.
    for s in list.iter().rev() {
        let now_q = crate::families::service::summary(s);
        let before: Option<qsi::service::Summary> =
            said.get(&format!("problem.{}", s.number)).and_then(decoded);
        if before.as_ref() == Some(&now_q) {
            continue;
        }
        let n = Noticed {
            problem: now_q.clone(),
            time: now,
        };
        let closed = now_q.status == qsi::service::ProblemStatus::Closed;
        let event = match &before {
            None if closed => {
                out.push(AnyEvent::Service(E::Opened(n.clone())));
                E::Closed(n)
            }
            None => E::Opened(n),
            Some(_) if closed => E::Closed(n),
            Some(b) if b.holds && !now_q.holds => E::Cleared(n),
            Some(_) => E::Seen(n),
        };
        out.push(AnyEvent::Service(event));
    }
    out
}

/// What the switch service has that the log did not say: each switch
/// made, changed or deleted.
pub fn switches(said: &Said, list: &[vswitchd::SwitchStatus], now: u64) -> Vec<AnyEvent> {
    let mut out = Vec::new();
    let before = said.switches();
    let mut names = BTreeSet::new();
    for s in list {
        let Some(switch) = v::switch(s) else { continue };
        names.insert(switch.name.clone());
        if before.get(&switch.name) != Some(&encoded(&switch)) {
            out.push(vres::Event::Switch(vres::SwitchChanged {
                name: switch.name.clone(),
                switch: Some(switch),
                time: now,
            }));
        }
    }
    for name in before.into_keys().filter(|n| !names.contains(n)) {
        out.push(vres::Event::Switch(vres::SwitchChanged {
            name,
            switch: None,
            time: now,
        }));
    }
    out.into_iter().map(AnyEvent::Vres).collect()
}

#[cfg(test)]
mod tests;
