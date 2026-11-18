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
//! - `confinelearn`: a call the confinement refuses is reported and
//!   fails, instead of ending the VMM (`crate::confine`).
//!
//! And on the control socket, `try <control|processor> <call>`: the
//! control thread, or a processor at its next exit, makes a call the
//! confinement forbids ([`forbidden`]), which must end the VMM.
//!
//! In any other build none of them exists: a restart ends the
//! partition, and the machine is the configuration's.

use std::sync::atomic::{AtomicU8, Ordering};

const HOLD_RESTART: u8 = 1;
const SMALL_PAGES: u8 = 2;
const NO_RNG: u8 = 4;
const NO_CONSOLE: u8 = 8;
const CONFINE_LEARN: u8 = 16;

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
            "confinelearn" => CONFINE_LEARN,
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

/// Whether a refused call is reported and failed, not fatal.
pub fn confine_learn() -> bool {
    on(CONFINE_LEARN)
}

/// The calls `try` may make: each one the confinement forbids every
/// thread of a VMM.
#[cfg(test_hooks)]
pub const CALLS: &[&str] = &[
    "open", "socket", "exec", "fork", "ptrace", "setuid", "mount", "kill", "unshare", "ioctl",
    "bpf", "chroot",
];

/// The call a processor makes at its next exit, when one is asked:
/// an index into [`CALLS`] plus one.
#[cfg(test_hooks)]
static PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// `try <control|processor> <call>` on the control socket.
#[cfg(all(test_hooks, target_os = "linux"))]
pub fn try_request(rest: &str) -> String {
    let mut words = rest.split_whitespace();
    let (Some(place), Some(call)) = (words.next(), words.next()) else {
        return "refused try <control|processor> <call>".into();
    };
    let Some(index) = CALLS.iter().position(|c| *c == call) else {
        return format!("refused no call {call}");
    };
    match place {
        "control" => format!("tried {}", forbidden(call)),
        "processor" => {
            PENDING.store(index + 1, Ordering::Relaxed);
            "ok at the next exit".into()
        }
        _ => format!("refused no place {place}"),
    }
}

/// The call a processor was asked to make, taken once.
#[cfg(test_hooks)]
pub fn pending() -> Option<&'static str> {
    match PENDING.swap(0, Ordering::Relaxed) {
        0 => None,
        n => CALLS.get(n - 1).copied(),
    }
}

/// Make the forbidden call `call`: under the confinement the kernel
/// ends the process before it returns. What it returned, when it did.
#[cfg(all(test_hooks, target_os = "linux"))]
pub fn forbidden(call: &str) -> String {
    let r = crate::sys::forbidden(call);
    format!("{call} returned {r}")
}
