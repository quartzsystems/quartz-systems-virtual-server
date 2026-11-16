// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the services say, from their sockets, as text a person reads.
//! A service that does not answer is said so.

use std::fmt::Write as _;
use std::path::PathBuf;

use partd::Calls as _;
use poold::Calls as _;
use vswitchd::Calls as _;

use crate::take::Sources;

/// The services' sockets.
#[derive(Clone, Debug)]
pub struct Live {
    /// The partition manager's.
    pub partd: PathBuf,
    /// The pool service's.
    pub poold: PathBuf,
    /// The switch service's.
    pub vswitchd: PathBuf,
}

fn down(what: &str, e: &dyn std::fmt::Display) -> String {
    format!("The {what} did not answer: {e}\n")
}

impl Sources for Live {
    fn partitions(&mut self) -> String {
        let mut c = partd::Client::new(&self.partd);
        let list = match c.list() {
            Ok(l) => l,
            Err(e) => return down("partition manager", &e),
        };
        let mut out = String::new();
        for s in &list {
            let _ = writeln!(out, "== {} {}\n{s:#?}", s.id, s.name);
            if let Ok(Ok(d)) = c.definition(s.id) {
                let _ = writeln!(out, "-- definition\n{d:#?}");
            }
            for name in c
                .profiles(s.id)
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default()
            {
                if let Ok(Ok(p)) = c.profile(s.id, name.clone()) {
                    let _ = writeln!(out, "-- profile {name}\n{p:#?}");
                }
            }
            if let Ok(Ok(h)) = c.history(s.id) {
                let _ = writeln!(out, "-- history");
                for p in h {
                    let _ = writeln!(out, "{} {:?} {} {}", p.time, p.source, p.code, p.text);
                }
            }
        }
        if let Ok(r) = c.resources() {
            let _ = writeln!(out, "== resources\n{r:#?}");
        }
        if let Ok(s) = c.settings() {
            let _ = writeln!(out, "== settings\n{s:#?}");
        }
        out
    }

    fn pools(&mut self) -> String {
        let mut c = poold::Client::new(&self.poold);
        let pools = match c.pools() {
            Ok(p) => p,
            Err(e) => return down("pool service", &e),
        };
        let mut out = format!("== pools\n{pools:#?}\n");
        if let Ok(d) = c.disks() {
            let _ = writeln!(out, "== disks\n{d:#?}");
        }
        if let Ok(v) = c.vdisks(String::new()) {
            let _ = writeln!(out, "== virtual disks\n{v:#?}");
        }
        if let Ok(i) = c.images() {
            let _ = writeln!(out, "== optical images\n{i:#?}");
        }
        if let Ok(v) = c.volumes() {
            let _ = writeln!(out, "== tape volumes\n{v:#?}");
        }
        out
    }

    fn switches(&mut self) -> String {
        let mut c = vswitchd::Client::new(&self.vswitchd);
        let switches = match c.switches() {
            Ok(s) => s,
            Err(e) => return down("switch service", &e),
        };
        let mut out = format!("== switches\n{switches:#?}\n");
        if let Ok(p) = c.ports(String::new()) {
            let _ = writeln!(out, "== ports\n{p:#?}");
        }
        if let Ok(i) = c.interfaces() {
            let _ = writeln!(out, "== interfaces\n{i:#?}");
        }
        out
    }
}
