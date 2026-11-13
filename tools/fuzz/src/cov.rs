// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! LLVM's `SanitizerCoverage` inline 8-bit counters: one byte for
//! every edge of the instrumented code, which the code itself counts
//! in. The instrumentation calls the one function here once at start
//! with where the counters are. A build without the instrumentation
//! never calls it, and the fuzzer then runs blind: it still mutates
//! and still catches a panic, it only cannot tell which input was new.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

static START: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static LEN: AtomicUsize = AtomicUsize::new(0);

/// Called by the instrumented code before `main`: the counters are
/// the bytes from `start` to `stop`.
///
/// # Safety
/// `start` and `stop` bound one writable array that lives as long as
/// the process, as LLVM's instrumentation passes them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sanitizer_cov_8bit_counters_init(start: *mut u8, stop: *mut u8) {
    if start.is_null() || stop <= start {
        return;
    }
    // SAFETY: both point into the same array, by the caller's contract.
    let len = usize::try_from(unsafe { stop.offset_from(start) }).unwrap_or(0);
    // A statically linked program has one such array; were there
    // several, the largest is the program's own.
    if len > LEN.load(Ordering::Relaxed) {
        START.store(start, Ordering::Relaxed);
        LEN.store(len, Ordering::Relaxed);
    }
}

/// How many counters there are; none in a build without them.
pub fn len() -> usize {
    LEN.load(Ordering::Relaxed)
}

/// Whether there are none.
pub fn is_empty() -> bool {
    len() == 0
}

/// Set every counter to zero.
pub fn reset() {
    let (start, len) = (START.load(Ordering::Relaxed), len());
    if !start.is_null() {
        // SAFETY: `start` and `len` are the array the instrumentation
        // gave; only the fuzzer's one thread reads or clears it between
        // runs, and the code under test only increments single bytes.
        unsafe { std::ptr::write_bytes(start, 0, len) };
    }
}

/// Look at the counters as they stand.
pub fn with<T>(look: impl FnOnce(&[u8]) -> T) -> T {
    let (start, len) = (START.load(Ordering::Relaxed), len());
    if start.is_null() {
        return look(&[]);
    }
    // SAFETY: as in `reset`; the slice does not outlive this call.
    look(unsafe { std::slice::from_raw_parts(start, len) })
}
