// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The clock at IPL: the system clock is set from the RTC, which keeps
//! UTC. The network service keeps it in step afterwards.

use std::fs;

use rustix::time::{ClockId, Timespec, clock_settime};

/// The first RTC's time, in seconds since the epoch.
const RTC: &str = "/sys/class/rtc/rtc0/since_epoch";

/// Set the system clock from the RTC. The time set, or why not.
pub fn set_from_rtc() -> Result<i64, String> {
    let text = fs::read_to_string(RTC).map_err(|e| format!("{RTC}: {e}"))?;
    let secs: i64 = text
        .trim()
        .parse()
        .map_err(|_| format!("{RTC}: `{}` is not a time", text.trim()))?;
    let at = Timespec {
        tv_sec: secs,
        tv_nsec: 0,
    };
    clock_settime(ClockId::Realtime, at).map_err(|e| format!("the clock: {e}"))?;
    Ok(secs)
}
