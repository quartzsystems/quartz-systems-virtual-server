// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The control queue's messages and what they do to the ports: the
//! ports announced by name, each side's open and close.
//!
//! A message is eight bytes, little-endian: the port's ID (`u32`), the
//! event (`u16`), a value (`u16`); a port's name follows its header in
//! the same buffer. What the guest sends is read as hostile: a message
//! shorter than its header, an event the device does not take, a port
//! it does not have, are passed over. What waits for the guest is
//! bounded: one of each kind a port, never more.

use std::collections::VecDeque;

/// The bytes of a message's header.
pub const HEADER: usize = 8;

/// The guest's driver is ready: the device announces its ports.
pub const DEVICE_READY: u16 = 0;
/// The device has a port.
pub const DEVICE_ADD: u16 = 1;
/// The guest has set a port up.
pub const PORT_READY: u16 = 3;
/// A port is the guest's console.
pub const CONSOLE_PORT: u16 = 4;
/// The console's size changed.
pub const RESIZE: u16 = 5;
/// A side opened (1) or closed (0) a port.
pub const PORT_OPEN: u16 = 6;
/// A port's name follows the header.
pub const PORT_NAME: u16 = 7;

/// The most messages that wait for the guest's buffers.
pub const MAX_WAITING: usize = 64;

/// One message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    /// The port's ID.
    pub id: u32,
    /// The event.
    pub event: u16,
    /// Its value.
    pub value: u16,
}

impl Message {
    /// A message from the guest's bytes; `None` when they are fewer
    /// than a header. What follows the header is passed over.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        let header = bytes.get(..HEADER)?;
        Some(Self {
            id: u32::from_le_bytes([header[0], header[1], header[2], header[3]]),
            event: u16::from_le_bytes([header[4], header[5]]),
            value: u16::from_le_bytes([header[6], header[7]]),
        })
    }

    /// Its bytes, with `extra` after the header.
    pub fn bytes(self, extra: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + extra.len());
        out.extend_from_slice(&self.id.to_le_bytes());
        out.extend_from_slice(&self.event.to_le_bytes());
        out.extend_from_slice(&self.value.to_le_bytes());
        out.extend_from_slice(extra);
        out
    }
}

/// What the device knows of one port.
#[derive(Clone, Debug, Default)]
struct Port {
    name: String,
    /// The guest set it up.
    ready: bool,
    /// The guest has it open.
    guest_open: bool,
    /// What the guest was last told of the host's side.
    told_open: Option<bool>,
}

/// The ports and the messages waiting for the guest.
#[derive(Debug)]
pub struct Control {
    /// The ports, from ID 1: ID 0 is not used.
    ports: Vec<Port>,
    driver_ready: bool,
    waiting: VecDeque<(Message, Vec<u8>)>,
    /// How many times a guest opened or closed a port.
    pub changes: u64,
    /// Messages of the guest's that were passed over.
    pub passed_over: u64,
}

impl Control {
    /// Ports named `names`, with IDs from 1.
    pub fn new(names: &[&str]) -> Self {
        Self {
            ports: names
                .iter()
                .map(|n| Port {
                    name: (*n).to_string(),
                    ..Port::default()
                })
                .collect(),
            driver_ready: false,
            waiting: VecDeque::new(),
            changes: 0,
            passed_over: 0,
        }
    }

    /// The ID of the port at `index`.
    pub fn id(index: usize) -> u32 {
        u32::try_from(index + 1).unwrap_or(u32::MAX)
    }

    fn index(&self, id: u32) -> Option<usize> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        (index < self.ports.len()).then_some(index)
    }

    /// The port count the configuration says: the named ones and port 0.
    pub fn max_ports(&self) -> u32 {
        Self::id(self.ports.len())
    }

    /// Queue a message for the guest. One of a kind for a port is kept:
    /// a newer one replaces it, so nothing grows without bound.
    fn queue(&mut self, message: Message, extra: Vec<u8>) {
        if let Some(at) = self
            .waiting
            .iter()
            .position(|(m, _)| m.id == message.id && m.event == message.event)
        {
            self.waiting[at] = (message, extra);
            return;
        }
        if self.waiting.len() < MAX_WAITING {
            self.waiting.push_back((message, extra));
        }
    }

    /// A message from the guest. `open` says whether the host's side of
    /// each port is open, by index.
    pub fn from_guest(&mut self, bytes: &[u8], open: &dyn Fn(usize) -> bool) {
        let Some(m) = Message::read(bytes) else {
            self.passed_over += 1;
            return;
        };
        match m.event {
            DEVICE_READY if m.value == 1 => {
                self.driver_ready = true;
                for index in 0..self.ports.len() {
                    let id = Self::id(index);
                    self.queue(
                        Message {
                            id,
                            event: DEVICE_ADD,
                            value: 0,
                        },
                        Vec::new(),
                    );
                }
            }
            PORT_READY => {
                let Some(index) = self.index(m.id) else {
                    self.passed_over += 1;
                    return;
                };
                if m.value != 1 {
                    // The guest could not set the port up.
                    self.ports[index].ready = false;
                    return;
                }
                self.ports[index].ready = true;
                let name = self.ports[index].name.clone().into_bytes();
                self.queue(
                    Message {
                        id: m.id,
                        event: PORT_NAME,
                        value: 1,
                    },
                    name,
                );
                let host = open(index);
                self.ports[index].told_open = Some(host);
                self.queue(
                    Message {
                        id: m.id,
                        event: PORT_OPEN,
                        value: u16::from(host),
                    },
                    Vec::new(),
                );
            }
            PORT_OPEN => {
                let Some(index) = self.index(m.id) else {
                    self.passed_over += 1;
                    return;
                };
                let now = m.value == 1;
                if self.ports[index].guest_open != now {
                    self.changes += 1;
                }
                self.ports[index].guest_open = now;
            }
            // The console's size, and whatever else a guest sends, is
            // not the device's concern.
            _ => self.passed_over += 1,
        }
    }

    /// The host's side of the port at `index` is open or not: the guest
    /// is told when it has set the port up and was told otherwise.
    pub fn host(&mut self, index: usize, open: bool) {
        let Some(port) = self.ports.get_mut(index) else {
            return;
        };
        if !port.ready || port.told_open == Some(open) {
            return;
        }
        port.told_open = Some(open);
        self.queue(
            Message {
                id: Self::id(index),
                event: PORT_OPEN,
                value: u16::from(open),
            },
            Vec::new(),
        );
    }

    /// The next message for the guest, as bytes.
    pub fn take(&mut self) -> Option<Vec<u8>> {
        self.waiting.pop_front().map(|(m, extra)| m.bytes(&extra))
    }

    /// Put a message back at the front: the guest had no buffer for it.
    pub fn unread(&mut self, bytes: &[u8]) {
        if let Some(m) = Message::read(bytes) {
            self.waiting.push_front((m, bytes[HEADER..].to_vec()));
            self.waiting.truncate(MAX_WAITING);
        }
    }

    /// Whether messages wait.
    pub fn pending(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// Whether the guest has the port at `index` open.
    pub fn guest_open(&self, index: usize) -> bool {
        self.ports.get(index).is_some_and(|p| p.guest_open)
    }

    /// The guest's driver was reset: every port is as before it was set
    /// up, and nothing waits.
    pub fn reset(&mut self) {
        for port in &mut self.ports {
            port.ready = false;
            port.guest_open = false;
            port.told_open = None;
        }
        self.driver_ready = false;
        self.waiting.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: u32, event: u16, value: u16) -> Vec<u8> {
        Message { id, event, value }.bytes(&[])
    }

    fn drain(c: &mut Control) -> Vec<(Message, Vec<u8>)> {
        std::iter::from_fn(|| c.take())
            .map(|b| (Message::read(&b).unwrap(), b[HEADER..].to_vec()))
            .collect()
    }

    #[test]
    fn the_ports_are_announced_by_name_with_the_host_s_side() {
        let mut c = Control::new(&["svc.0", "con.0"]);
        assert_eq!(c.max_ports(), 3);
        c.from_guest(&msg(0, DEVICE_READY, 1), &|_| false);
        let adds = drain(&mut c);
        assert_eq!(
            adds.iter()
                .map(|(m, _)| (m.id, m.event))
                .collect::<Vec<_>>(),
            [(1, DEVICE_ADD), (2, DEVICE_ADD)]
        );
        c.from_guest(&msg(2, PORT_READY, 1), &|i| i == 1);
        let said = drain(&mut c);
        assert_eq!(said[0].0.event, PORT_NAME);
        assert_eq!(said[0].1, b"con.0");
        assert_eq!(
            said[1].0,
            Message {
                id: 2,
                event: PORT_OPEN,
                value: 1
            }
        );
        // The host's side goes and comes back: the guest is told once
        // each, and only of a port it set up.
        c.host(1, false);
        c.host(1, false);
        c.host(0, true);
        assert_eq!(
            drain(&mut c)
                .iter()
                .map(|(m, _)| (m.id, m.value))
                .collect::<Vec<_>>(),
            [(2, 0)]
        );
        c.from_guest(&msg(2, PORT_OPEN, 1), &|_| true);
        assert!(c.guest_open(1));
        assert!(!c.guest_open(0));
        c.from_guest(&msg(2, PORT_OPEN, 0), &|_| true);
        assert_eq!(c.changes, 2);
    }

    #[test]
    fn what_a_guest_sends_wrong_is_passed_over_and_nothing_grows() {
        let mut c = Control::new(&["svc.0"]);
        c.from_guest(&[1, 2, 3], &|_| true);
        c.from_guest(&msg(0, PORT_READY, 1), &|_| true);
        c.from_guest(&msg(9, PORT_OPEN, 1), &|_| true);
        c.from_guest(&msg(u32::MAX, PORT_READY, 1), &|_| true);
        c.from_guest(&msg(1, RESIZE, 7), &|_| true);
        c.from_guest(&msg(1, 0xFFFF, 0), &|_| true);
        assert_eq!(c.passed_over, 6);
        assert!(!c.pending());
        // A guest that says ready over and over has one of each waiting.
        for _ in 0..10_000 {
            c.from_guest(&msg(0, DEVICE_READY, 1), &|_| true);
            c.from_guest(&msg(1, PORT_READY, 1), &|_| true);
        }
        assert_eq!(drain(&mut c).len(), 3);
        let mut big = Control::new(&["p"; 200]);
        big.from_guest(&msg(0, DEVICE_READY, 1), &|_| true);
        assert_eq!(drain(&mut big).len(), MAX_WAITING);
        // A message the guest had no room for goes back first.
        c.host(0, false);
        let first = c.take().unwrap();
        c.unread(&first);
        assert_eq!(c.take().unwrap(), first);
        c.reset();
        c.host(0, true);
        assert!(!c.pending());
    }
}
