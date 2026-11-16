// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The one log every connection publishes from: what the services said,
//! translated, numbered and kept.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use qsi::any::AnyEvent;
use qsisess::{EventLog, Limits};

use super::{Restored, Store};

/// The log and its file.
#[derive(Debug)]
pub struct Hub {
    log: EventLog,
    store: Option<Store>,
    first: u64,
}

/// The hub, shared by the pumps and the connections.
pub type Shared = Arc<Mutex<Hub>>;

/// Lock the shared hub, a poisoned lock being taken as it is.
pub fn lock(hub: &Shared) -> MutexGuard<'_, Hub> {
    hub.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hub {
    /// The log taken up from `store`.
    #[must_use]
    pub fn kept(store: Store, restored: Restored) -> Self {
        Self {
            first: restored.first,
            log: restored.log,
            store: Some(store),
        }
    }

    /// A log kept nowhere.
    #[must_use]
    pub fn memory(limits: &Limits) -> Self {
        Self {
            log: EventLog::new(limits),
            store: None,
            first: 1,
        }
    }

    /// Push an event: numbered, kept, and published by each
    /// connection at its next turn. One too long for a message is
    /// dropped, said by the caller.
    ///
    /// # Errors
    /// The event is longer than a message may be.
    pub fn push(&mut self, e: &AnyEvent) -> Result<u64, qsisess::Error> {
        let seq = self.log.push_any(e)?;
        if let Some(store) = &mut self.store
            && let Some(event) = self.log.from(seq).next()
        {
            store.append(event);
        }
        Ok(seq)
    }

    /// The log.
    #[must_use]
    pub fn log(&self) -> &EventLog {
        &self.log
    }

    /// A console's `since`, as this log takes it.
    #[must_use]
    pub fn since(&self, since: Option<u64>) -> Option<u64> {
        super::store::place(self.first, since)
    }

    /// Bring the file to the disk.
    pub fn sync(&mut self) {
        if let Some(store) = &mut self.store {
            store.sync();
        }
    }
}
