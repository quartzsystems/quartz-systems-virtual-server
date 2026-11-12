// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Reaping. PID 1 is the parent of its services and of every orphan.

use rustix::process::{WaitOptions, WaitStatus, wait};

/// How a child ended, in words.
fn how(status: WaitStatus) -> String {
    match (status.exit_status(), status.terminating_signal()) {
        (Some(code), _) => format!("exit {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "ended".to_string(),
    }
}

/// Every child that has ended: its process and how it ended.
pub fn ended() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    while let Ok(Some((pid, status))) = wait(WaitOptions::NOHANG) {
        let pid = u32::try_from(pid.as_raw_nonzero().get()).unwrap_or(0);
        out.push((pid, how(status)));
    }
    out
}
