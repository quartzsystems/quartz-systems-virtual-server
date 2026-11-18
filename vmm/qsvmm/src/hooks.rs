// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The VMM's test hooks, a test image's alone (`cfg(test_hooks)`), for
//! the soak of `docs/PHASE11.md` (Section 1). Words in the file
//! `/run/qsvshook.vmm`, which the harness writes, or in `QSVS_HOOK_VMM`
//! for a developer's run:
//!
//! - `holdrestart`: a guest that restarts is held with its memory, as
//!   after a fault, so a kernel that panicked can be dumped.
//! - `smallpages`: the guest's memory is of ordinary pages.
//! - `norng`, `noconsole`: the partition has no virtio-rng, no
//!   virtio-console.
//!
//! In any other build none of them exists: a restart ends the
//! partition, and the machine is the configuration's.

use std::sync::atomic::{AtomicU8, Ordering};

const HOLD_RESTART: u8 = 1;
const SMALL_PAGES: u8 = 2;
const NO_RNG: u8 = 4;
const NO_CONSOLE: u8 = 8;

static ON: AtomicU8 = AtomicU8::new(0);

/// Read the hooks once, as root, before anything is built.
#[cfg(test_hooks)]
pub fn read() {
    let mut words = std::fs::read_to_string("/run/qsvshook.vmm").unwrap_or_default();
    words.push(' ');
    words.push_str(&std::env::var("QSVS_HOOK_VMM").unwrap_or_default());
    let mut on = 0;
    for word in words.split_whitespace() {
        on |= match word {
            "holdrestart" => HOLD_RESTART,
            "smallpages" => SMALL_PAGES,
            "norng" => NO_RNG,
            "noconsole" => NO_CONSOLE,
            _ => 0,
        };
    }
    if on != 0 {
        eprintln!("qsvmm: test hooks: {}", words.trim());
    }
    ON.store(on, Ordering::Relaxed);
}

/// Without the hooks there is nothing to read.
#[cfg(not(test_hooks))]
pub fn read() {}

fn on(bit: u8) -> bool {
    ON.load(Ordering::Relaxed) & bit != 0
}

/// Whether a restart of the guest holds the partition.
pub fn hold_restart() -> bool {
    on(HOLD_RESTART)
}

/// Whether the guest's memory is of ordinary pages, whatever the
/// configuration says.
pub fn small_pages() -> bool {
    on(SMALL_PAGES)
}

/// Whether the partition goes without its virtio-rng.
pub fn no_rng() -> bool {
    on(NO_RNG)
}

/// Whether the partition goes without its virtio-console.
pub fn no_console() -> bool {
    on(NO_CONSOLE)
}
