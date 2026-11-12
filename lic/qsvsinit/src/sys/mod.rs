// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What PID 1 asks of the kernel directly: reaping, the power button,
//! the panic path. Signals come through `supervise::signal`.

pub mod button;
pub mod panic;
pub mod reap;
