// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Each uplink's carrier, looked at every turn of the loop. An
//! interface that loses it posts `A2008025` when the bond still has
//! another, `A2008020` when the uplink has none left; one that gets it
//! back posts `C2008020`. Each names the interface by its location
//! code, and each is an event. An uplink found without carrier at the
//! start is said once it has been so for the grace below.

use vswitch::Switch;

use super::build::named;
use super::{Core, Env};
use crate::kernel::Kernel;
use crate::proto::{Event, Member, uplink_state};

/// How long an uplink may be without carrier at the start before it is
/// said: links come up a moment after the IPL.
pub const START_GRACE_SECS: u64 = 30;

impl<K: Kernel, E: Env> Core<K, E> {
    /// The interfaces of `s`'s uplink as they are now.
    pub(crate) fn members(&mut self, s: &Switch) -> Vec<Member> {
        let Some(u) = &s.uplink else {
            return Vec::new();
        };
        let nics = self.env.nics();
        let links = self.links();
        u.interfaces
            .iter()
            .map(|location| {
                let nic = nics.iter().find(|n| n.location == *location);
                let link = nic.and_then(|n| named(&links, &n.name));
                Member {
                    location: location.clone(),
                    name: nic.map(|n| n.name.clone()).unwrap_or_default(),
                    carrier: link.is_some_and(|l| l.carrier),
                    speed_mbps: nic.and_then(|n| n.speed_mbps).unwrap_or(0),
                }
            })
            .collect()
    }

    /// One of [`uplink_state`] for `s`.
    pub(crate) fn uplink_state(s: &Switch, members: &[Member]) -> &'static str {
        if s.uplink.is_none() {
            return uplink_state::NONE;
        }
        let there = members.iter().filter(|m| !m.name.is_empty()).count();
        let up = members.iter().filter(|m| m.carrier).count();
        if there == 0 {
            uplink_state::MISSING
        } else if up == 0 {
            uplink_state::DOWN
        } else if up < members.len() {
            uplink_state::DEGRADED
        } else {
            uplink_state::UP
        }
    }

    /// Take the uplinks' carrier as it is, saying nothing: at the start.
    pub(crate) fn watch_quietly(&mut self) {
        let switches = self.state.switches.clone();
        self.carrier.clear();
        for s in &switches {
            for m in self.members(s) {
                if m.carrier {
                    self.carrier.insert((s.id, m.location), true);
                }
            }
        }
    }

    /// Look at every uplink's carrier and say what changed. `since` is
    /// how long the service has run.
    pub fn watch(&mut self, since: u64) {
        let switches = self.state.switches.clone();
        for s in &switches {
            let members = self.members(s);
            let state = Self::uplink_state(s, &members);
            let up = members.iter().filter(|m| m.carrier).count();
            for m in &members {
                let key = (s.id, m.location.clone());
                let was = self.carrier.get(&key).copied();
                let now = m.carrier;
                let changed = match was {
                    Some(was) => was != now,
                    // Never seen with carrier: said once the grace is over.
                    None => now || since >= START_GRACE_SECS,
                };
                if !changed {
                    continue;
                }
                self.carrier.insert(key, now);
                let detail = format!("{}, {}", s.name, m.location);
                // The interface by its location code, which the switch's
                // uplink names it by.
                let about = [
                    refcode::about::interface(&m.location, &m.location),
                    refcode::about::switch(&s.name),
                ];
                if now {
                    if was == Some(false) {
                        self.env
                            .post_about(&refcode::UPLINK_RETURNED, &detail, &about);
                    }
                } else if up > 0 {
                    self.env
                        .post_about(&refcode::UPLINK_MEMBER_DOWN, &detail, &about);
                } else {
                    self.env.post_about(&refcode::UPLINK_LOST, &detail, &about);
                }
                if was.is_some() || !now {
                    let time = self.env.now();
                    self.publish(&Event::Uplink {
                        switch: s.name.clone(),
                        location: m.location.clone(),
                        carrier: now,
                        state: state.to_string(),
                        time,
                    });
                }
            }
        }
    }

    /// One turn of the loop: the ports whose VMMs ended are forgotten,
    /// and the uplinks looked at.
    pub fn turn(&mut self, since: u64) {
        self.forget_gone();
        self.watch(since);
    }
}
