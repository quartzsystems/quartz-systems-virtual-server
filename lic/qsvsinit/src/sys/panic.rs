// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A panic in PID 1 would end it, and the kernel with it. The hook
//! posts the fatal reference code and parks instead.

use std::time::Duration;

use crate::console::Console;

/// Install the hook. The first thing PID 1 does.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let mut console = Console::open();
        console.line(&refcode::PANIC.line());
        console.line(&format!("{info}"));
        // Never return: the abort that follows a panic must not happen.
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    }));
}
