// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Stopping a partition's processors and letting them go on
//! (`docs/VMM.md`). Each processor looks at the pause before it enters
//! the guest and parks while there is a reason to; the one who holds
//! kicks them out of the guest until all have parked. A processor that
//! parks leaves its registers, which a dump reads. One that goes on
//! tells KVM it was stopped, so the guest's clock does not take the time
//! for a lockup.
//!
//! The reasons are apart: a partition kept for a dump after a fault
//! stays stopped when its pool has room again.

use std::os::unix::thread::JoinHandleExt;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use dump::Cpu;
use kvm_ioctls::VcpuFd;
use vmm_sys_util::signal::Killable;

use crate::sys::Kick;

/// How long a hold waits for every processor to park.
pub const PARK_WAIT: Duration = Duration::from_secs(2);
const STEP: Duration = Duration::from_millis(10);

/// Why the processors are stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// A write found the pool full.
    Space,
    /// A dump is being written.
    Dump,
    /// The partition is kept for a dump: a fault, or a terminal code.
    Kept,
}

impl Reason {
    fn bit(self) -> u8 {
        match self {
            Reason::Space => 1,
            Reason::Dump => 2,
            Reason::Kept => 4,
        }
    }
}

#[derive(Default)]
struct State {
    reasons: u8,
    parked: usize,
    /// Each processor's registers when it last parked.
    cpus: Vec<Option<Cpu>>,
}

/// The processors' pause.
#[derive(Default)]
pub struct Pause {
    state: Mutex<State>,
    changed: Condvar,
    threads: Mutex<Vec<Kick>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pause {
    /// A processor's thread, to be kicked.
    pub fn register(&self, thread: &JoinHandle<()>) {
        let mut threads = lock(&self.threads);
        threads.push(Kick(thread.as_pthread_t()));
        let count = threads.len();
        lock(&self.state).cpus.resize(count, None);
    }

    /// Whether the processors are to stop.
    pub fn paused(&self) -> bool {
        lock(&self.state).reasons != 0
    }

    /// Whether the partition is kept.
    pub fn kept(&self) -> bool {
        lock(&self.state).reasons & Reason::Kept.bit() != 0
    }

    /// Stop every processor for `why`; returns when all have parked, or
    /// after `wait`. Whether all did.
    pub fn hold_for(&self, why: Reason, wait: Duration) -> bool {
        lock(&self.state).reasons |= why.bit();
        let threads = lock(&self.threads).clone();
        let mut waited = Duration::ZERO;
        let mut state = lock(&self.state);
        while state.parked < threads.len() && waited < wait {
            drop(state);
            for t in &threads {
                let _ = t.kill(super::vcpu::kick_signal());
            }
            state = lock(&self.state);
            state = self
                .changed
                .wait_timeout(state, STEP)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            waited += STEP;
        }
        state.parked >= threads.len()
    }

    /// Stop every processor for `why`, waiting the usual time.
    pub fn hold(&self, why: Reason) -> bool {
        self.hold_for(why, PARK_WAIT)
    }

    /// Stop every processor from one of them, which parks next: the
    /// others are kicked and nothing is waited for.
    pub fn hold_from_processor(&self, why: Reason) {
        lock(&self.state).reasons |= why.bit();
        for t in lock(&self.threads).iter() {
            let _ = t.kill(super::vcpu::kick_signal());
        }
    }

    /// Take `why` away; the processors go on when no reason is left.
    pub fn release(&self, why: Reason) {
        let mut state = lock(&self.state);
        state.reasons &= !why.bit();
        drop(state);
        self.changed.notify_all();
    }

    /// The registers each processor left when it parked.
    pub fn cpus(&self) -> Vec<Cpu> {
        lock(&self.state)
            .cpus
            .iter()
            .map(|c| c.unwrap_or_default())
            .collect()
    }

    /// Processor `index`'s thread: wait here while the processors are
    /// held, its registers left for a dump.
    pub fn park(&self, vcpu: &VcpuFd, index: u32) {
        let cpu = crate::dump::cpu_of(vcpu);
        let mut state = lock(&self.state);
        if let Some(slot) = state.cpus.get_mut(index as usize) {
            *slot = Some(cpu);
        }
        state.parked += 1;
        self.changed.notify_all();
        while state.reasons != 0 {
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        state.parked -= 1;
        drop(state);
        // The guest's clock is told the processor was stopped.
        let _ = vcpu.kvmclock_ctrl();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reason_is_its_own_and_the_last_one_lets_them_go() {
        let pause = Pause::default();
        assert!(!pause.paused());
        // No processor: every one has parked at once.
        assert!(pause.hold(Reason::Kept));
        assert!(pause.hold(Reason::Space));
        assert!(pause.kept());
        pause.release(Reason::Space);
        assert!(pause.paused(), "kept still");
        pause.hold_from_processor(Reason::Dump);
        pause.release(Reason::Kept);
        assert!(pause.paused());
        assert!(!pause.kept());
        pause.release(Reason::Dump);
        assert!(!pause.paused());
        assert!(pause.cpus().is_empty());
    }
}
