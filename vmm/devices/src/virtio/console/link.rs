// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A port's link to the host: the bytes each way, bounded, and whether
//! the host's side is connected. The device works one end on its
//! worker's thread and never waits; the host's end is worked by
//! threads of the VMM's own that may wait, so a slow reader on the host
//! stalls the port and nothing else.
//!
//! Toward the guest, the host's writer waits while the buffer is full;
//! toward the host, the device stops taking the guest's buffers while
//! it is, and the guest's writes wait in its own queue.

use std::collections::VecDeque;
use std::io;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use vmm_sys_util::eventfd::EventFd;

/// The most bytes held each way.
pub const CAPACITY: usize = 64 * 1024;

#[derive(Debug, Default)]
struct State {
    to_guest: VecDeque<u8>,
    to_host: VecDeque<u8>,
    connected: bool,
    guest_open: bool,
    /// Counts connections, so a writer of an old one stops.
    connection: u64,
}

/// One port's link.
#[derive(Debug)]
pub struct Link {
    state: Mutex<State>,
    changed: Condvar,
    /// Written when the device has something to do.
    wake: EventFd,
}

impl Link {
    /// A link with nothing connected.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            wake: EventFd::new(0)?,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn poke(&self) {
        let _ = self.wake.write(1);
        self.changed.notify_all();
    }

    /// The eventfd the device's worker listens on.
    pub fn wake_fd(&self) -> io::Result<EventFd> {
        self.wake.try_clone()
    }

    // The host's end.

    /// A peer connected: what was held for the last one is dropped. Its
    /// connection's number.
    pub fn connect(&self) -> u64 {
        let mut s = self.lock();
        s.to_guest.clear();
        s.to_host.clear();
        s.connected = true;
        s.connection += 1;
        let n = s.connection;
        drop(s);
        self.poke();
        n
    }

    /// The peer of connection `n` went away.
    pub fn disconnect(&self, n: u64) {
        let mut s = self.lock();
        if s.connection != n {
            return;
        }
        s.connected = false;
        s.to_guest.clear();
        s.to_host.clear();
        drop(s);
        self.poke();
    }

    /// Bytes from the peer of connection `n` for the guest. Waits while
    /// the buffer is full; bytes the guest has its port closed for are
    /// dropped. `false` when the connection is gone.
    pub fn to_guest(&self, n: u64, mut bytes: &[u8]) -> bool {
        let mut s = self.lock();
        while !bytes.is_empty() {
            if s.connection != n || !s.connected {
                return false;
            }
            if !s.guest_open {
                return true;
            }
            let room = CAPACITY.saturating_sub(s.to_guest.len());
            if room == 0 {
                s = self
                    .changed
                    .wait_timeout(s, Duration::from_millis(200))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            let take = room.min(bytes.len());
            s.to_guest.extend(&bytes[..take]);
            bytes = &bytes[take..];
            let _ = self.wake.write(1);
        }
        true
    }

    /// Bytes from the guest for the peer of connection `n`: waits until
    /// there are some. `None` when the connection is gone.
    pub fn from_guest(&self, n: u64, most: usize) -> Option<Vec<u8>> {
        let mut s = self.lock();
        loop {
            if s.connection != n || !s.connected {
                return None;
            }
            if !s.to_host.is_empty() {
                let take = most.min(s.to_host.len());
                let out: Vec<u8> = s.to_host.drain(..take).collect();
                drop(s);
                // The device may take the guest's buffers again.
                let _ = self.wake.write(1);
                return Some(out);
            }
            s = self
                .changed
                .wait_timeout(s, Duration::from_millis(200))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    // The device's end, which never waits.

    /// Whether a peer is connected.
    pub fn connected(&self) -> bool {
        self.lock().connected
    }

    /// The guest opened or closed the port; closed, what waits for it is
    /// dropped.
    pub fn set_guest_open(&self, open: bool) {
        let mut s = self.lock();
        s.guest_open = open;
        if !open {
            s.to_guest.clear();
        }
        drop(s);
        self.changed.notify_all();
    }

    /// Up to `most` bytes for the guest.
    pub fn take_for_guest(&self, most: usize) -> Vec<u8> {
        let mut s = self.lock();
        let take = most.min(s.to_guest.len());
        let out = s.to_guest.drain(..take).collect();
        drop(s);
        self.changed.notify_all();
        out
    }

    /// How many of the guest's bytes wait for the peer.
    pub fn waiting_for_host(&self) -> usize {
        self.lock().to_host.len()
    }

    /// Whether bytes wait for the guest.
    pub fn has_for_guest(&self) -> bool {
        !self.lock().to_guest.is_empty()
    }

    /// Whether the device may take another of the guest's buffers: the
    /// peer has room, or there is no peer and what the guest sends is
    /// dropped.
    pub fn takes_from_guest(&self) -> bool {
        let s = self.lock();
        !s.connected || s.to_host.len() < CAPACITY
    }

    /// The guest's bytes for the peer; dropped when there is none.
    pub fn put_from_guest(&self, bytes: &[u8]) {
        let mut s = self.lock();
        if s.connected {
            s.to_host.extend(bytes);
        }
        drop(s);
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn bytes_go_each_way_bounded_and_only_while_connected() {
        let link = Arc::new(Link::new().unwrap());
        // Nothing connected: the guest's bytes are dropped.
        link.put_from_guest(b"lost");
        let n = link.connect();
        assert!(link.connected());
        // The guest has the port closed: bytes for it are dropped.
        assert!(link.to_guest(n, b"dropped"));
        assert!(!link.has_for_guest());
        link.set_guest_open(true);
        assert!(link.to_guest(n, b"hello"));
        assert_eq!(link.take_for_guest(3), b"hel");
        assert_eq!(link.take_for_guest(100), b"lo");
        link.put_from_guest(b"frame");
        assert_eq!(link.from_guest(n, 64).unwrap(), b"frame");

        // A full buffer stops the guest's side, not anything else.
        link.put_from_guest(&vec![0; CAPACITY]);
        assert!(!link.takes_from_guest());
        assert_eq!(link.from_guest(n, CAPACITY).unwrap().len(), CAPACITY);
        assert!(link.takes_from_guest());

        // A writer that waits for room goes on when the device takes.
        let writer = {
            let link = link.clone();
            thread::spawn(move || link.to_guest(n, &vec![1; CAPACITY + 10]))
        };
        while link.take_for_guest(CAPACITY).is_empty() {}
        assert!(writer.join().unwrap());
        assert!(link.take_for_guest(CAPACITY).len() >= 10);

        // A new connection: the old one's writer stops, nothing is kept.
        link.put_from_guest(b"old");
        let m = link.connect();
        assert!(link.from_guest(n, 10).is_none());
        assert!(!link.to_guest(n, b"x"));
        link.disconnect(n);
        assert!(link.connected(), "an old connection's end changes nothing");
        link.disconnect(m);
        assert!(!link.connected());
        assert!(link.from_guest(m, 10).is_none());
        assert!(link.takes_from_guest());
    }
}
