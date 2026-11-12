// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service that fails on demand: with the `flaky-fails` hook on it
//! ends at once every time it is started; without, it says it is ready
//! and does nothing.

use std::time::Duration;

/// Run the service.
pub fn run() -> ! {
    if crate::on(crate::FLAKY_FAILS) {
        eprintln!("failing, as the {} hook asks", crate::FLAKY_FAILS);
        std::process::exit(3);
    }
    if let Err(e) = supervise::ready::signal() {
        eprintln!("not ready: {e}");
        std::process::exit(1);
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
