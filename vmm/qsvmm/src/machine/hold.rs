// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition held for its pool's space. A write to a thin virtual
//! disk that finds the pool full waits, and the partition's processors
//! stop: the guest sees nothing of it but time that passed. `qsvmm`
//! reports `A2005160` when the first write waits and `C2005160` when the
//! last that waited is done; the partition stays Running throughout.
//! The write is tried again each second, so the partition goes on by
//! itself when the pool has room.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use devices::virtio::scsi::lun::Hold;

use super::pause::Pause;

/// How long a write that found no room waits before it is tried again.
pub const RETRY: Duration = Duration::from_secs(1);

/// The hold of one partition's disks.
pub struct Held {
    pause: Arc<Pause>,
    waiting: Mutex<usize>,
    retry: Duration,
}

impl Held {
    /// The hold over the processors `pause` stops.
    pub fn new(pause: Arc<Pause>) -> Self {
        Self {
            pause,
            waiting: Mutex::new(0),
            retry: RETRY,
        }
    }
}

impl Hold for Held {
    fn wait(&self, first: bool) {
        if first {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            *waiting += 1;
            if *waiting == 1 {
                crate::post(
                    &refcode::PARTITION_HELD,
                    "a write found the storage pool full",
                );
                drop(waiting);
                self.pause.hold();
            }
        }
        thread::sleep(self.retry);
    }

    fn done(&self) {
        let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
        *waiting = waiting.saturating_sub(1);
        if *waiting == 0 {
            crate::post(&refcode::PARTITION_RESUMED, "the storage pool has room");
            self.pause.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_processors_go_on_when_the_last_write_is_done() {
        let pause = Arc::new(Pause::default());
        let held = Held {
            pause: pause.clone(),
            waiting: Mutex::new(0),
            retry: Duration::ZERO,
        };
        held.wait(true);
        held.wait(true);
        held.wait(false);
        assert!(pause.paused());
        held.done();
        assert!(pause.paused(), "one write still waits");
        held.done();
        assert!(!pause.paused());
    }
}
