// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The test hooks of a QS/VS test image. A release image has none of
//! this: the build puts the `qsvshook` binary and the hooks of PID 1
//! into a test image only, and the manifest says which kind an image
//! is.
//!
//! - [`proto`]: the lines the harness and the channel exchange on the
//!   second serial port.
//! - [`churn`]: the state churn of the power-pull test, and the check
//!   of what it left.
//! - [`channel`], [`flaky`]: the two services (Linux).
//!
//! A hook is switched on for one IPL by an SMBIOS OEM string the
//! harness gives QEMU, [`oem_string`].

#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
pub mod channel;
pub mod churn;
#[cfg(target_os = "linux")]
pub mod flaky;
pub mod proto;

/// The hook that keeps a slot on trial from confirming (PID 1's).
pub const REFUSE_CONFIRM: &str = "refuse-confirm";
/// The hook that makes the `flaky` service fail every time it starts.
pub const FLAKY_FAILS: &str = "flaky-fails";
/// The hook that churns the state files from the start of the IPL.
pub const CHURN: &str = "churn";

/// The SMBIOS OEM string that switches the hook `name` on:
/// `qsvs.hook=<name>`.
pub fn oem_string(name: &str) -> String {
    format!("{}hook={name}", release::KEY_PREFIX)
}

/// Whether the hook `name` is on for this IPL.
pub fn on(name: &str) -> bool {
    hostinv::oem_strings(&hostinv::Roots::default()).contains(&oem_string(name))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_hook_is_named_under_the_product_s_key_prefix() {
        assert_eq!(super::oem_string("churn"), "qsvs.hook=churn");
    }
}
