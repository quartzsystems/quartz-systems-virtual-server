// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's console (`docs/SERVICE.md`): the screen kept whole
//! from the console port whether anyone watches, and one viewer at a
//! time. With the profile's console mode serial, the viewer has the
//! serial port's bytes instead, which the socket's service relays.

use std::sync::mpsc::SyncSender;

use console_port::Shown;
use model::ConsoleMode;

use super::{Manager, Port, System};
use crate::proto::{ConsoleIn, ConsoleOut, Refused};

/// The most messages that wait for a viewer: a viewer that does not take
/// them is detached.
pub const VIEWER_QUEUE: usize = 512;

impl<S: System> Manager<S> {
    /// Whether `id`'s console is the serial port: its running profile's
    /// mode, or its default profile's.
    pub(crate) fn console_mode(&self, id: u32) -> Result<bool, Refused> {
        let p = self.partition(id)?;
        let name = p
            .run
            .as_ref()
            .map_or(&p.definition.default_profile, |r| &r.profile);
        Ok(p.profiles
            .get(name)
            .is_some_and(|profile| profile.boot.console == ConsoleMode::Serial))
    }

    /// Bytes from the console port: the screen kept, the viewer sent
    /// what changed, a whole frame asked for when one did not fit.
    pub(crate) fn console_bytes(&mut self, id: u32, data: &[u8]) {
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        let (shown, ask) = p.console.bytes(data);
        let mut gone = false;
        if let Some((_, viewer)) = &p.viewer {
            for s in shown {
                let out = match s {
                    Shown::Frame(frame) => ConsoleOut::Frame { frame },
                    Shown::Update(update) => ConsoleOut::Update { update },
                };
                if viewer.try_send(out).is_err() {
                    gone = true;
                    break;
                }
            }
        }
        if gone && let Some((viewer, _)) = p.viewer.take() {
            p.console.detach(viewer);
        }
        if ask {
            self.system
                .send(id, Port::Console, console_port::Console::refresh());
        }
    }

    /// Attach a viewer of `id`'s console, sent to `to`: taken over from
    /// another when `take_over`. Its number and whether it is the serial
    /// port's.
    pub fn console_attach(
        &mut self,
        id: u32,
        take_over: bool,
        to: SyncSender<ConsoleOut>,
    ) -> Result<(u64, bool), Refused> {
        let serial = self.console_mode(id)?;
        let p = self
            .partitions
            .get_mut(&id)
            .ok_or_else(|| vec![qvsmsg::PARTITION_MISSING.with(&[&id.to_string()])])?;
        let name = p.definition.name.clone();
        let Ok((viewer, displaced)) = p.console.attach(take_over) else {
            return Err(vec![qvsmsg::CONSOLE_IN_USE.with(&[&name])]);
        };
        if let Some((_, old)) = p.viewer.take()
            && displaced.is_some()
        {
            let _ = old.try_send(ConsoleOut::Ended {
                why: "another viewer took the console over".into(),
            });
        }
        if !serial {
            match p.console.frame() {
                Some(frame) => {
                    let _ = to.try_send(ConsoleOut::Frame { frame });
                }
                None => self
                    .system
                    .send(id, Port::Console, console_port::Console::refresh()),
            }
        }
        if let Some(p) = self.partitions.get_mut(&id) {
            p.viewer = Some((viewer, to));
        }
        Ok((viewer, serial))
    }

    /// What viewer `viewer` of `id` sent: keys and refreshes for the
    /// console port. Whether it still has the console.
    pub fn console_input(&mut self, id: u32, viewer: u64, input: &ConsoleIn) -> bool {
        let Some(p) = self.partitions.get(&id) else {
            return false;
        };
        if p.console.viewer() != Some(viewer) {
            return false;
        }
        match input {
            ConsoleIn::Key { key } => {
                if let Some(frame) = p.console.key(viewer, *key) {
                    self.system.send(id, Port::Console, frame);
                }
            }
            ConsoleIn::Refresh => {
                self.system
                    .send(id, Port::Console, console_port::Console::refresh());
            }
            // The serial port's bytes are the socket's service's.
            ConsoleIn::Bytes { .. } => {}
        }
        true
    }

    /// Viewer `viewer` of `id` detached.
    pub fn console_detach(&mut self, id: u32, viewer: u64) {
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        p.console.detach(viewer);
        if p.viewer.as_ref().is_some_and(|(v, _)| *v == viewer) {
            p.viewer = None;
        }
    }

    /// Whether viewer `viewer` still has `id`'s console.
    pub fn console_viewer(&self, id: u32, viewer: u64) -> bool {
        self.partitions
            .get(&id)
            .is_some_and(|p| p.console.viewer() == Some(viewer))
    }
}
