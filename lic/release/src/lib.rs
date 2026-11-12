// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's identity over QS/400's `vrm` crate: the release, the build
//! ID, the product text, the banner, the prefixes everything named
//! after the product starts with, and the [`Appliance`] description the
//! product-neutral base crates take.

#![forbid(unsafe_code)]

pub use appliance::{Appliance, Event};
pub use vrm::Release;

/// The release `build.rs` captured: `VERSION` (`1.0.0` is `V1R0M0`), or
/// `QSVS_RELEASE`.
pub const RELEASE: Release = vrm::release_from_env!("QSVS_");

/// The build ID: `git describe`, or `QSVS_BUILD_ID`.
pub const BUILD_ID: &str = env!("QSVS_BUILD_ID");

/// The product's short name.
pub const NAME: &str = "QS/VS";

/// The product text.
pub const PRODUCT: &str = "Quartz Systems/Virtual Server";

/// Prefix of a fix identifier, `QVnnnnn`.
pub const FIX_PREFIX: &str = "QV";

/// Prefix of a message identifier.
pub const MSG_PREFIX: &str = "QVS";

/// Prefix of every environment variable QS/VS reads.
pub const ENV_PREFIX: &str = "QSVS_";

/// Prefix of QS/VS's kernel command-line keys.
pub const KEY_PREFIX: &str = "qsvs.";

/// The lower-case identifier: PID 1 is `qsvsinit`.
pub const ID: &str = "qsvs";

/// Where `QSVSDATA` is mounted.
pub const DATA_DIR: &str = "/data";

/// The run directory.
pub const RUN_DIR: &str = "/run";

/// PID 1's control socket.
pub const CONTROL_SOCKET: &str = "/run/qsvsinit.sock";

/// Where the service binaries are in the initramfs.
pub const BIN_DIR: &str = "/qsvs/bin";

/// What a default host name starts with.
pub const HOST_PREFIX: &str = "QSVS";

/// The FAT volume label of a seed volume.
pub const SEED_LABEL: &str = "QSVSSEED";

/// QS/VS as the base crates see it. The caller says how an event is
/// posted and a fact noted: PID 1 does both itself, a service asks
/// PID 1.
pub const fn appliance(post: fn(&Event<'_>), note: fn(&str, &str)) -> Appliance {
    Appliance {
        id: ID,
        name: NAME,
        banner,
        data_dir: DATA_DIR,
        run_dir: RUN_DIR,
        host_prefix: HOST_PREFIX,
        seed_label: SEED_LABEL,
        post,
        note,
    }
}

/// The keys of the facts PID 1 keeps, as the control socket's status
/// names them.
pub mod fact {
    /// The product's name.
    pub const PRODUCT: &str = "product";
    /// The release, `V1R0M0`.
    pub const RELEASE: &str = "release";
    /// The build ID.
    pub const BUILD: &str = "build";
    /// `release` or `test`: whether the test hooks are in the image.
    pub const IMAGE: &str = "image";
    /// The slot this IPL came from: `A`, `B`, `media` or `unknown`.
    pub const SLOT: &str = "slot";
    /// The A/B record, or why there is none.
    pub const RECORD: &str = "record";
    /// The host name.
    pub const HOST: &str = "host";
    /// The system serial number.
    pub const SERIAL: &str = "serial";
    /// The management address, or the reason there is none.
    pub const MANAGEMENT: &str = appliance::NOTE_MANAGEMENT;
    /// Seconds since the epoch.
    pub const TIME: &str = "time";
    /// The date and time as displayed.
    pub const DATE: &str = "date";
    /// The current reference code and its text.
    pub const REFERENCE: &str = "reference";
    /// `/data`: the device mounted there, or `memory`.
    pub const DATA: &str = "data";
    /// `yes` once every service has been ready.
    pub const READY: &str = "ready";
    /// What the KVM probe found.
    pub const KVM: &str = "kvm";
}

/// The status screen's text: PID 1 draws it, the conformance harness
/// reads it.
pub mod screen {
    /// The line before a status screen on the serial port.
    pub const BEGIN: &str = "---- status ----";
    /// The line after it.
    pub const END: &str = "---- end ----";
    /// The slot row's label.
    pub const SLOT: &str = "Slot";
    /// The host name row's label.
    pub const HOST: &str = "Host name";
    /// The management address row's label.
    pub const MANAGEMENT: &str = "Management";
    /// The date and time row's label.
    pub const DATE: &str = "Date and time";
    /// The reference code row's label.
    pub const REFERENCE: &str = "Reference";
    /// The label of a service's row.
    pub const SERVICE: &str = "Service";

    /// One row: the label in its column, then the value.
    pub fn row(label: &str, value: &str) -> String {
        format!("{label:<14} {value}")
    }

    /// A service's row: its name in its column, then its state.
    pub fn service(name: &str, state: &str) -> String {
        row(SERVICE, &format!("{name:<10} {state}"))
    }
}

/// The banner for a release and a build ID: `QS/VS V1R0M0 a1b2c3d`.
pub fn banner_of(release: Release, build_id: &str) -> String {
    format!("{NAME} {release} {build_id}")
}

/// This build's banner.
pub fn banner() -> String {
    banner_of(RELEASE, BUILD_ID)
}

/// A fix identifier, `QV00001`.
pub fn fix_id(number: u32) -> String {
    format!("{FIX_PREFIX}{number:05}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_names_the_release_and_the_build() {
        let r = Release { v: 1, r: 0, m: 0 };
        assert_eq!(banner_of(r, "abc1234"), "QS/VS V1R0M0 abc1234");
        assert!(banner().starts_with("QS/VS V"));
        assert!(banner().ends_with(BUILD_ID));
    }

    #[test]
    fn the_appliance_description_is_qsvs() {
        let a = appliance(|_| {}, |_, _| {});
        assert_eq!(a.control_socket().to_str(), Some(CONTROL_SOCKET));
        assert_eq!(a.state_dir().to_str(), Some("/data/state"));
        assert_eq!(a.log_socket().to_str(), Some("/run/logd.sock"));
        assert_eq!((a.banner)(), banner());
        // A FAT label is eleven characters at most.
        assert!(a.seed_label.len() <= 11);
    }

    #[test]
    fn fixes_are_qv_and_five_digits() {
        assert_eq!(fix_id(1), "QV00001");
        assert_eq!(fix_id(12345), "QV12345");
    }
}
