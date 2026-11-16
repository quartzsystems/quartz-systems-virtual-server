// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A stream's far end on the host: a console's viewer, a dump being
//! read, an image being uploaded. Each is the service's connection,
//! read by a thread of its own so the session never waits on it.

use qsisess::Note;

/// What a pipe has for the console now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Bytes to send.
    Bytes(Vec<u8>),
    /// Nothing yet.
    Idle,
    /// No more: the stream closes, or is reset with a message.
    End(Option<Note>),
}

/// A stream's far end.
pub trait Pipe: Send {
    /// What there is for the console, without waiting.
    fn poll(&mut self) -> Flow;
    /// Bytes the console sent. A message refuses them: the stream is
    /// reset with it.
    ///
    /// # Errors
    /// The far end will take no more.
    fn put(&mut self, bytes: &[u8]) -> Result<(), Note>;
    /// The console will send no more.
    fn closed(&mut self);
    /// The stream is over, either way: let go of the far end.
    fn reset(&mut self) {}
}

/// What opens a stream's far end.
pub trait Opener: Send {
    /// Attach to partition `id`'s console: whether it is the serial
    /// port's, and its pipe.
    ///
    /// # Errors
    /// The partition manager's refusal.
    fn console(&mut self, id: u32, take_over: bool) -> Result<(bool, Box<dyn Pipe>), Vec<Note>>;
    /// Read partition `id`'s dump `name`: its size and its pipe.
    ///
    /// # Errors
    /// The partition manager's refusal.
    fn dump(&mut self, id: u32, name: &str) -> Result<(u64, Box<dyn Pipe>), Vec<Note>>;
    /// Read the host dump `name`: its size and its pipe.
    ///
    /// # Errors
    /// No such dump.
    fn host_dump(&mut self, name: &str) -> Result<(u64, Box<dyn Pipe>), Vec<Note>>;
    /// Stage update image `name` of exactly `bytes` bytes: its pipe.
    /// The update service says how it ended as its events.
    ///
    /// # Errors
    /// The update service's refusal.
    fn stage(
        &mut self,
        name: &str,
        bytes: u64,
        allow_older: bool,
    ) -> Result<Box<dyn Pipe>, Vec<Note>>;
    /// Upload image `name` of exactly `bytes` bytes: its pipe, which
    /// says how it ended as an event.
    ///
    /// # Errors
    /// The pool service's refusal.
    fn upload(&mut self, name: &str, bytes: u64) -> Result<Box<dyn Pipe>, Vec<Note>>;
}
