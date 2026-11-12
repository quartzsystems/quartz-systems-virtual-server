// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The test hooks of PID 1, compiled under `cfg(test_hooks)` only: a
//! test image has them, a release image does not. A hook is switched
//! on for one IPL by an SMBIOS OEM string, `qsvs.hook=<name>`, which
//! the harness gives QEMU; the image itself is the same every IPL.

use std::time::Duration;

use hostinv::Roots;
use supervise::Service;

/// The hook that keeps a slot on trial from confirming.
pub const REFUSE_CONFIRM: &str = "refuse-confirm";

/// The test hooks' binary.
const BINARY: &str = "/qsvs/bin/qsvshook";

/// A service that fails on demand.
pub const FLAKY: Service = Service {
    name: "flaky",
    binary: BINARY,
    args: &["flaky"],
    order: 80,
    needs: &[],
    ready_within: Duration::from_secs(10),
};

/// The harness's way to the control socket: the second serial port.
pub const CHANNEL: Service = Service {
    name: "hook",
    binary: BINARY,
    args: &["channel"],
    order: 90,
    needs: &[],
    ready_within: Duration::from_secs(10),
};

/// Whether the hook `name` is on for this IPL.
pub fn has(name: &str) -> bool {
    let wanted = format!("{}hook={name}", release::KEY_PREFIX);
    hostinv::oem_strings(&Roots::default()).contains(&wanted)
}
