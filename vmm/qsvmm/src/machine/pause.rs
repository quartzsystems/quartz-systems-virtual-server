// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Stopping a partition's processors and letting them go on: a
//! partition held for its pool's space (`docs/VMM.md`). Each processor
//! looks at the flag before it enters the guest and parks while it is
//! set; the one who holds kicks them out of the guest until all have
//! parked. A processor that goes on tells KVM it was stopped, so the
//! guest's clock does not take the time for a lockup.

use std::os::unix::thread::JoinHandleExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use kvm_ioctls::VcpuFd;
use vmm_sys_util::signal::Killable;

use crate::sys::Kick;

/// How long a hold waits for every processor to park.
const PARK_WAIT: Duration = Duration::from_secs(2);
const STEP: Duration = Duration::from_millis(10);

/// The processors' pause.
#[derive(Default)]
pub struct Pause {
    paused: AtomicBool,
    parked: Mutex<usize>,
    changed: Condvar,
    threads: Mutex<Vec<Kick>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pause {
    /// A processor's thread, to be kicked.
    pub fn register(&self, thread: &JoinHandle<()>) {
        lock(&self.threads).push(Kick(thread.as_pthread_t()));
    }

    /// Whether the processors are to stop.
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    /// Stop every processor; returns when all have parked, or after a
    /// bounded wait.
    pub fn hold(&self) {
        self.paused.store(true, Ordering::Release);
        let threads = lock(&self.threads).clone();
        let mut waited = Duration::ZERO;
        let mut parked = lock(&self.parked);
        while *parked < threads.len() && waited < PARK_WAIT {
            for t in &threads {
                let _ = t.kill(super::vcpu::kick_signal());
            }
            parked = self
                .changed
                .wait_timeout(parked, STEP)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            waited += STEP;
        }
    }

    /// Let the processors go on.
    pub fn release(&self) {
        self.paused.store(false, Ordering::Release);
        let _guard = lock(&self.parked);
        self.changed.notify_all();
    }

    /// A processor's thread: wait here while the processors are held.
    pub fn park(&self, vcpu: &VcpuFd) {
        let mut parked = lock(&self.parked);
        *parked += 1;
        self.changed.notify_all();
        while self.paused() {
            parked = self
                .changed
                .wait_timeout(parked, Duration::from_millis(100))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        *parked -= 1;
        drop(parked);
        // The guest's clock is told the processor was stopped.
        let _ = vcpu.kvmclock_ctrl();
    }
}
