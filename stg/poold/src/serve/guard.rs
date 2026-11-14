// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A pool's set as the kernel's device, with one rule added: a transfer
//! that changed a unit's state does not complete until the label says
//! so on the disks that are left. A pool that is lost answers every
//! transfer with an error and writes nothing to its units.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use ublk::Target;

/// How long a transfer waits for the label.
const WAIT: Duration = Duration::from_secs(5);
/// `EIO`.
const EIO: i32 = 5;

/// The states the label on the disks has, how many of the host's
/// transfers are under way, and whether the pool is lost.
#[derive(Default)]
pub struct Recorded {
    states: Mutex<u64>,
    changed: Condvar,
    busy: AtomicUsize,
    lost: AtomicBool,
}

impl Recorded {
    /// The label was written with these states.
    pub fn set(&self, states: u64) {
        *self.states.lock().unwrap_or_else(PoisonError::into_inner) = states;
        self.changed.notify_all();
    }

    /// Whether the host has a transfer under way.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed) > 0
    }

    /// The pool is lost: nothing more is written to its units.
    pub fn lose(&self) {
        self.lost.store(true, Ordering::Release);
    }

    fn wait(&self, now: &dyn Fn() -> u64) {
        let mut recorded = self.states.lock().unwrap_or_else(PoisonError::into_inner);
        let mut waited = Duration::ZERO;
        let step = Duration::from_millis(20);
        while *recorded != now() && waited < WAIT {
            recorded = self
                .changed
                .wait_timeout(recorded, step)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            waited += step;
        }
    }
}

/// The set, guarded.
pub struct Guarded {
    inner: Box<dyn Target>,
    states: Arc<dyn Fn() -> u64 + Send + Sync>,
    recorded: Arc<Recorded>,
}

impl Guarded {
    /// `inner`, whose units' states `states` gives, waiting for
    /// `recorded`.
    pub fn new(
        inner: Box<dyn Target>,
        states: Arc<dyn Fn() -> u64 + Send + Sync>,
        recorded: Arc<Recorded>,
    ) -> Self {
        Self {
            inner,
            states,
            recorded,
        }
    }

    fn transfer(&self, run: impl FnOnce() -> Result<(), i32>) -> Result<(), i32> {
        if self.recorded.lost.load(Ordering::Acquire) {
            return Err(EIO);
        }
        self.recorded.busy.fetch_add(1, Ordering::Relaxed);
        let done = run();
        self.recorded.wait(&*self.states);
        self.recorded.busy.fetch_sub(1, Ordering::Relaxed);
        done
    }
}

impl Target for Guarded {
    fn blocks(&self) -> u64 {
        self.inner.blocks()
    }

    fn read(&self, block: u64, buf: &mut [u8]) -> Result<(), i32> {
        self.transfer(|| self.inner.read(block, buf))
    }

    fn write(&self, block: u64, buf: &[u8]) -> Result<(), i32> {
        self.transfer(|| self.inner.write(block, buf))
    }

    fn flush(&self) -> Result<(), i32> {
        self.transfer(|| self.inner.flush())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockdev::MemBlockDev;
    use ublk::BlockTarget;

    #[test]
    fn a_lost_pool_writes_nothing() {
        let recorded = Arc::new(Recorded::default());
        let g = Guarded::new(
            Box::new(BlockTarget(MemBlockDev::new(16))),
            Arc::new(|| 0),
            recorded.clone(),
        );
        let page = vec![7u8; 4096];
        assert_eq!(g.write(0, &page), Ok(()));
        recorded.lose();
        assert_eq!(g.write(1, &page), Err(EIO));
        let mut back = vec![0u8; 4096];
        assert_eq!(g.read(0, &mut back), Err(EIO));
    }
}
