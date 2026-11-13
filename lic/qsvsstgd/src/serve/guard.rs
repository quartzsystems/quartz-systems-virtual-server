// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The set as the kernel's device, with one rule added: a transfer
//! that changed a member's state does not complete until the label
//! says so on the disks that are left. A mirrored set has nothing on
//! its members that says one is stale; if the host went on writing to
//! one disk and lost power before the label knew, the next IPL could
//! take the disk that had failed for the good one.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use protect::set::MirrorSet;
use ublk::{BlockTarget, Target};

/// How long a transfer waits for the label. The loop that writes it
/// looks ten times a second; while it prepares a disk it looks later,
/// and a transfer then goes on without having waited for ever.
const WAIT: Duration = Duration::from_secs(5);

/// The members' states as one number: a state's byte a member.
pub fn states(set: &MirrorSet) -> u64 {
    set.members()
        .iter()
        .fold(0, |all, m| all << 8 | u64::from(m.state.as_u8()))
}

/// The states the label on the disks has, and how many of the host's
/// transfers are under way.
#[derive(Default)]
pub struct Recorded {
    states: Mutex<u64>,
    changed: Condvar,
    busy: AtomicUsize,
}

impl Recorded {
    /// The label was written with these states.
    pub fn set(&self, states: u64) {
        *self.states.lock().unwrap_or_else(PoisonError::into_inner) = states;
        self.changed.notify_all();
    }

    /// Whether the host has a transfer under way: a copy of one
    /// member to another then gives way to it.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed) > 0
    }

    /// Wait until the label has the states `now` gives, or [`WAIT`]
    /// has passed.
    fn wait(&self, now: impl Fn() -> u64) {
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
    set: MirrorSet,
    inner: BlockTarget<MirrorSet>,
    recorded: Arc<Recorded>,
}

impl Guarded {
    /// `set` as a device whose transfers wait for `recorded`.
    pub fn new(set: MirrorSet, recorded: Arc<Recorded>) -> Self {
        Self {
            inner: BlockTarget(set.clone()),
            set,
            recorded,
        }
    }

    /// Carry out one transfer of the host's. Whatever it did to the
    /// members is on the label before its answer goes to the kernel.
    fn transfer<T>(&self, run: impl FnOnce() -> T) -> T {
        self.recorded.busy.fetch_add(1, Ordering::Relaxed);
        let done = run();
        self.recorded.wait(|| states(&self.set));
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
    use blockdev::{BlockDev, MemBlockDev};
    use protect::UnitState;
    use protect::set::MirrorMember;
    use std::time::Instant;

    fn member(n: u8) -> MirrorMember {
        MirrorMember {
            uuid: [n; 16],
            state: UnitState::Active,
            dev: Some(Box::new(MemBlockDev::new(64)) as Box<dyn BlockDev>),
        }
    }

    #[test]
    fn a_transfer_goes_at_once_while_the_label_is_level_and_waits_when_it_is_not() {
        let set = MirrorSet::open([9; 16], 64, vec![member(1), member(2)]).unwrap();
        let recorded = Arc::new(Recorded::default());
        recorded.set(states(&set));
        let guarded = Arc::new(Guarded::new(set.clone(), Arc::clone(&recorded)));
        let block = vec![7u8; 4096];
        let started = Instant::now();
        guarded.write(0, &block).unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!recorded.busy());

        // A member is taken out: the label does not know yet.
        set.suspend(1).unwrap();
        assert_ne!(states(&set), 0x0101);
        let waiter = {
            let guarded = Arc::clone(&guarded);
            std::thread::spawn(move || {
                let started = Instant::now();
                guarded.write(1, &vec![8u8; 4096]).unwrap();
                started.elapsed()
            })
        };
        std::thread::sleep(Duration::from_millis(300));
        assert!(!waiter.is_finished());
        assert!(recorded.busy());
        // The loop writes the label and says so: the transfer completes.
        recorded.set(states(&set));
        let took = waiter.join().unwrap();
        assert!(took >= Duration::from_millis(250) && took < WAIT);
        let mut back = vec![0u8; 4096];
        guarded.read(1, &mut back).unwrap();
        assert_eq!(back[0], 8);
    }
}
