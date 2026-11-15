// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's console port on the host's side (`docs/SERVICE.md`).
//! The partition manager keeps the latest whole frame of the console,
//! applying the system's updates as they come whether or not anyone
//! watches, so a viewer that attaches gets the whole screen at once and
//! the updates after it.
//!
//! The frames, updates and keys are the forms of QS/400's protocol
//! crate. Here they are checked (the size, a cell for every place,
//! every run on the grid) and passed on; no attribute is read. An
//! update that does not fit the grid is dropped and a whole frame asked
//! for.
//!
//! One viewer at a time: a second is refused unless it takes over, and
//! the one it displaces is told.

use qsipart::console::{self, FromSystem, Grid, Key, ToSystem};
use qsipart::{Reader, caps};

/// What the host's side says it can do on the console port.
pub const HOST_CAPS: u64 = caps::CONSOLE_V1;
/// The most refreshes asked for in a row without a whole frame.
const MAX_ASKED: u32 = 3;

/// What a viewer is sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    /// A whole frame.
    Frame(console::Frame),
    /// What changed.
    Update(console::Update),
}

/// Why an attach was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Another viewer has it, and this one did not take over.
    InUse,
}

/// A partition's console.
#[derive(Debug, Default)]
pub struct Console {
    reader: Reader,
    grid: Grid,
    viewer: Option<u64>,
    next: u64,
    asked: u32,
    /// Messages dropped: a frame or update that failed its checks.
    pub dropped: u64,
}

impl Console {
    /// A console with nothing shown yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The host's hello, sent on each connection.
    pub fn peer_hello() -> Vec<u8> {
        ToSystem::Hello(console::Hello {
            version: qsipart::VERSION,
            caps: HOST_CAPS,
        })
        .encode()
        .unwrap_or_default()
    }

    /// Ask the system for a whole frame.
    pub fn refresh() -> Vec<u8> {
        ToSystem::Refresh.encode().unwrap_or_default()
    }

    /// A new connection: what was read of the last one is dropped, the
    /// screen is kept until a frame replaces it.
    pub fn connected(&mut self) {
        self.reader.reset();
        self.asked = 0;
    }

    /// Bytes from the system: what to show the viewer, and whether a
    /// whole frame is to be asked for.
    pub fn bytes(&mut self, data: &[u8]) -> (Vec<Shown>, bool) {
        let mut shown = Vec::new();
        let mut ask = false;
        self.reader.push(data);
        while let Some(f) = self.reader.next_frame() {
            let m = match FromSystem::decode(&f) {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(_) => {
                    self.dropped += 1;
                    ask = true;
                    continue;
                }
            };
            if self.grid.apply(&m).is_err() {
                self.dropped += 1;
                ask = true;
                continue;
            }
            match m {
                FromSystem::Hello(_) => {}
                FromSystem::Frame(frame) => {
                    self.asked = 0;
                    shown.push(Shown::Frame(frame));
                }
                FromSystem::Update(update) => shown.push(Shown::Update(update)),
            }
        }
        // Asked for a bounded number of times until a frame comes.
        if ask {
            ask = self.asked < MAX_ASKED;
            self.asked += 1;
        }
        (shown, ask)
    }

    /// The whole screen as it is now; none before the first frame.
    pub fn frame(&self) -> Option<console::Frame> {
        self.grid.ready().then(|| console::Frame {
            seq: self.grid.seq,
            rows: self.grid.rows,
            cols: self.grid.cols,
            cells: self.grid.cells.clone(),
            status: self.grid.status.clone(),
        })
    }

    /// The screen as text, a line a row.
    pub fn text(&self) -> String {
        self.grid.text()
    }

    /// Attach a viewer, taking over from another when `take_over`: its
    /// number, and the number of the viewer it displaced.
    pub fn attach(&mut self, take_over: bool) -> Result<(u64, Option<u64>), Refused> {
        if self.viewer.is_some() && !take_over {
            return Err(Refused::InUse);
        }
        self.next += 1;
        let displaced = self.viewer.replace(self.next);
        Ok((self.next, displaced))
    }

    /// The viewer `viewer` is gone.
    pub fn detach(&mut self, viewer: u64) {
        if self.viewer == Some(viewer) {
            self.viewer = None;
        }
    }

    /// The viewer that has the console.
    pub fn viewer(&self) -> Option<u64> {
        self.viewer
    }

    /// A key from `viewer`: its frame for the system, when that viewer
    /// has the console.
    pub fn key(&self, viewer: u64, key: Key) -> Option<Vec<u8>> {
        (self.viewer == Some(viewer))
            .then(|| ToSystem::Key(key).encode().ok())
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::{Cell, Run, Status, Update};

    fn frame(seq: u32, text: &str) -> console::Frame {
        let mut cells: Vec<Cell> = text.chars().map(|ch| Cell { ch, attr: 0 }).collect();
        cells.resize(24 * 80, Cell::default());
        console::Frame {
            seq,
            rows: 24,
            cols: 80,
            cells,
            status: Status::default(),
        }
    }

    #[test]
    fn the_screen_is_kept_whole_and_a_viewer_gets_it_at_once() {
        let mut c = Console::new();
        assert!(c.frame().is_none());
        let f = FromSystem::Frame(frame(1, "Sign On")).encode().unwrap();
        let (shown, ask) = c.bytes(&f);
        assert_eq!(shown.len(), 1);
        assert!(!ask);
        let u = FromSystem::Update(Update {
            seq: 2,
            runs: vec![Run {
                row: 1,
                col: 0,
                cells: vec![Cell { ch: 'X', attr: 0 }],
            }],
            status: Status::default(),
        })
        .encode()
        .unwrap();
        for piece in u.chunks(5) {
            c.bytes(piece);
        }
        let whole = c.frame().unwrap();
        assert_eq!(whole.seq, 2);
        assert_eq!(whole.cells[80].ch, 'X');
        assert!(c.text().starts_with("Sign On"));
    }

    #[test]
    fn an_update_off_the_grid_is_dropped_and_a_frame_asked_for() {
        let mut c = Console::new();
        // An update before any frame.
        let u = |row: u8| {
            FromSystem::Update(Update {
                seq: 1,
                runs: vec![Run {
                    row,
                    col: 79,
                    cells: vec![Cell::default(); 2],
                }],
                status: Status::default(),
            })
            .encode()
            .unwrap()
        };
        assert_eq!(c.bytes(&u(0)), (Vec::new(), true));
        c.bytes(&FromSystem::Frame(frame(1, "")).encode().unwrap());
        assert!(c.bytes(&u(0)).1);
        // Asked for a few times only, until a frame comes.
        assert!(c.bytes(&u(0)).1);
        assert!(c.bytes(&u(0)).1);
        assert!(!c.bytes(&u(0)).1);
        assert_eq!(c.dropped, 5);
        // A frame that fails its checks is the protocol's to refuse.
        let mut bad = frame(3, "");
        bad.cells.pop();
        let bytes = qsipart::frame::encode(console::kind::FRAME, &ipc_body(&bad)).unwrap();
        assert!(c.bytes(&bytes).0.is_empty());
    }

    fn ipc_body(f: &console::Frame) -> Vec<u8> {
        // The frame's body as the system would write it, unchecked.
        let encoded = FromSystem::Frame(console::Frame {
            cells: f.cells.clone(),
            ..frame(f.seq, "")
        });
        let mut whole = encoded.encode().unwrap();
        whole.drain(..qsipart::frame::HEADER);
        // One cell fewer: its six bytes, and the list's count less one.
        let count_at = 4 + 1 + 1;
        let n = u32::from_le_bytes(whole[count_at..count_at + 4].try_into().unwrap()) - 1;
        whole[count_at..count_at + 4].copy_from_slice(&n.to_le_bytes());
        let end = count_at + 4 + 6 * n as usize;
        whole.drain(end..end + 6);
        whole
    }

    #[test]
    fn one_viewer_at_a_time_and_a_take_over_displaces() {
        let mut c = Console::new();
        let (first, none) = c.attach(false).unwrap();
        assert_eq!(none, None);
        assert_eq!(c.attach(false), Err(Refused::InUse));
        assert!(c.key(first, Key::Enter).is_some());
        let (second, displaced) = c.attach(true).unwrap();
        assert_eq!(displaced, Some(first));
        assert!(c.key(first, Key::Enter).is_none());
        assert!(c.key(second, Key::Char('a')).is_some());
        c.detach(first);
        assert_eq!(c.viewer(), Some(second));
        c.detach(second);
        assert!(c.attach(false).is_ok());
    }
}
