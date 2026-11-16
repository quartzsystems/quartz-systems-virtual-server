// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The one log every connection publishes from: what the services said,
//! translated, numbered and kept.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use qsi::any::AnyEvent;
use qsisess::{EventLog, Limits};

use super::{Said, Store};

/// The log and its file.
#[derive(Debug)]
pub struct Hub {
    log: EventLog,
    store: Option<Store>,
    said: Said,
}

/// The hub, shared by the pumps and the connections.
pub type Shared = Arc<Mutex<Hub>>;

/// Lock the shared hub, a poisoned lock being taken as it is.
pub fn lock(hub: &Shared) -> MutexGuard<'_, Hub> {
    hub.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hub {
    /// The log taken up from `store`, with what it last said.
    #[must_use]
    pub fn kept(store: Store, log: EventLog, said: Said) -> Self {
        Self {
            log,
            store: Some(store),
            said,
        }
    }

    /// A log kept nowhere.
    #[must_use]
    pub fn memory(limits: &Limits) -> Self {
        Self {
            log: EventLog::new(limits),
            store: None,
            said: Said::default(),
        }
    }

    /// Push an event: numbered, kept, and published by each
    /// connection at its next turn. One that says only what the log
    /// said already is not pushed (`None`). One too long for a message
    /// is dropped, said by the caller.
    ///
    /// # Errors
    /// The event is longer than a message may be.
    pub fn push(&mut self, e: &AnyEvent) -> Result<Option<u64>, qsisess::Error> {
        if !self.said.note(e) {
            return Ok(None);
        }
        let seq = self.log.push_any(e)?;
        if let Some(store) = &mut self.store
            && let Some(event) = self.log.from(seq).next()
        {
            store.append(event);
        }
        Ok(Some(seq))
    }

    /// What the log last said.
    #[must_use]
    pub fn said(&self) -> &Said {
        &self.said
    }

    /// Take up a source: its first look after a start is compared with
    /// what the log said, and what differs pushed. A source never
    /// compared before has its look taken as said, with nothing pushed.
    pub fn take_up(&mut self, source: &str, events: &[AnyEvent]) {
        let known = self.said.knows(source);
        for e in events {
            if known {
                let _ = self.push(e);
            } else {
                self.said.note(e);
            }
        }
        self.said.known(source);
    }

    /// The log.
    #[must_use]
    pub fn log(&self) -> &EventLog {
        &self.log
    }

    /// Bring the file to the disk.
    pub fn sync(&mut self) {
        if let Some(store) = &mut self.store {
            store.sync();
        }
        self.said.sync();
    }
}
