// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The switch service itself: the switches as state has them, the
//! partitions' ports, each uplink's carrier, and what each request does.
//! It does nothing to the machine itself: it asks a [`Kernel`] and an
//! [`Env`], so all of it runs in a test.
//!
//! - [`build`]: a switch's bridge and uplink, built from state and kept
//!   as state says.
//! - [`ports`]: a partition's port: its tap, its rules, its addresses.
//! - [`requests`]: the socket's requests.
//! - [`watch`]: each uplink's carrier, every turn of the loop.

mod build;
mod ports;
mod requests;
mod watch;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

use refcode::RefCode;
use vswitch::Switches;
use vswitch::port::PortSpec;

use crate::kernel::{KLink, Kernel};
use crate::proto::{Adapter, Event, Nic};

/// What the service asks of the host beside the kernel.
pub trait Env {
    /// Seconds since the epoch.
    fn now(&self) -> u64;
    /// Post a reference code through PID 1.
    fn post(&mut self, code: &'static RefCode, detail: &str);
    /// A line for the host's log.
    fn log(&mut self, line: &str);
    /// The host's physical network interfaces, from its inventory.
    fn nics(&mut self) -> Vec<Nic>;
    /// The device the management network is on, as the network service
    /// says; none when it has none or does not answer.
    fn management(&mut self) -> Option<String>;
    /// Every Ethernet adapter of every partition's profiles, as the
    /// partition manager says; an error when it does not answer.
    fn adapters(&mut self) -> Result<Vec<Adapter>, String>;
    /// Keep the switches: the state file, written atomically.
    fn save(&mut self, switches: &Switches) -> Result<(), String>;
    /// Keep the ports' specifications in the run directory, for a
    /// service started again.
    fn save_ports(&mut self, ports: &[PortSpec]);
}

/// A partition's port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Port {
    /// What it was opened as.
    pub spec: PortSpec,
    /// The switch's number.
    pub switch: u32,
    /// Its tap.
    pub tap: String,
}

/// The service.
pub struct Core<K: Kernel, E: Env> {
    pub(crate) kernel: K,
    pub(crate) env: E,
    /// The switches as kept.
    pub(crate) state: Switches,
    /// The partitions' ports, by partition and slot.
    pub(crate) ports: BTreeMap<(u32, u32), Port>,
    /// Why a switch was not built, by its number.
    pub(crate) failures: BTreeMap<u32, String>,
    /// The carrier last seen of each uplink interface: by the switch's
    /// number and the location code.
    pub(crate) carrier: BTreeMap<(u32, String), bool>,
    subscribers: Vec<Sender<Event>>,
}

impl<K: Kernel, E: Env> Core<K, E> {
    /// The service over `kernel` and `env`, with the switches `state`.
    pub fn new(kernel: K, env: E, state: Switches) -> Self {
        Self {
            kernel,
            env,
            state,
            ports: BTreeMap::new(),
            failures: BTreeMap::new(),
            carrier: BTreeMap::new(),
            subscribers: Vec::new(),
        }
    }

    /// The host as the service works on it.
    pub fn env(&mut self) -> &mut E {
        &mut self.env
    }

    /// The switches as kept.
    pub fn switches(&self) -> &Switches {
        &self.state
    }

    /// Every link, or none when the kernel does not say.
    pub(crate) fn links(&mut self) -> Vec<KLink> {
        match self.kernel.links() {
            Ok(links) => links,
            Err(e) => {
                self.env.log(&format!("the links: {e}"));
                Vec::new()
            }
        }
    }

    /// Add a subscriber: every event from now on is sent to it.
    pub fn subscribe(&mut self, subscriber: Sender<Event>) {
        self.subscribers.push(subscriber);
    }

    /// Send an event to every subscriber still there, and say it in
    /// the log.
    pub(crate) fn publish(&mut self, event: &Event) {
        let line = match event {
            Event::Switch { name, status, .. } => match status {
                Some(_) => format!("switch {name}: made or changed"),
                None => format!("switch {name}: deleted"),
            },
            Event::Uplink {
                switch,
                location,
                carrier,
                state,
                ..
            } => format!(
                "switch {switch}: uplink {location} {}, the uplink {state}",
                if *carrier {
                    "has carrier"
                } else {
                    "lost carrier"
                }
            ),
            Event::Port {
                partition,
                slot,
                switch,
                open,
                ..
            } => format!(
                "partition {partition} slot {slot}: port on {switch} {}",
                if *open { "opened" } else { "closed" }
            ),
        };
        self.env.log(&line);
        self.subscribers.retain(|s| s.send(event.clone()).is_ok());
    }

    /// Keep the ports' specifications.
    pub(crate) fn keep_ports(&mut self) {
        let specs: Vec<PortSpec> = self.ports.values().map(|p| p.spec.clone()).collect();
        self.env.save_ports(&specs);
    }

    /// Build every switch from state and take up the ports the kernel
    /// still has: at the service's start. `kept` is what the run
    /// directory says of the ports; with `sweep`, what this service made
    /// and state no longer has is taken away. What is posted says how it
    /// went.
    pub fn start(&mut self, kept: &[PortSpec], sweep: bool) {
        let switches = self.state.switches.clone();
        let mut built = 0;
        for s in &switches {
            match self.build(s) {
                Ok(()) => built += 1,
                Err(e) => {
                    self.env
                        .post(&refcode::SWITCH_NOT_BUILT, &format!("{}: {e}", s.name));
                    self.failures.insert(s.id, e);
                }
            }
        }
        if sweep {
            self.sweep();
        }
        let found = self.adopt(kept);
        self.watch_quietly();
        self.env.post(
            &refcode::VSWITCHD_STARTED,
            &format!(
                "{} switch(es), {built} built; {found} partition port(s) found",
                switches.len()
            ),
        );
    }
}
