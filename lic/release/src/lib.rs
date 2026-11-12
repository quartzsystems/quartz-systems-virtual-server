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
    /// Who has the serial port: noted by the Service Tools console.
    pub const SERIAL_CONSOLE: &str = "console.serial";
    /// [`SERIAL_CONSOLE`] while a session is on the serial port.
    pub const SERIAL_SESSION: &str = "session";
    /// [`SERIAL_CONSOLE`] when the port is PID 1's again.
    pub const SERIAL_IDLE: &str = "idle";
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
    /// The last line: how the console is reached.
    pub const SIGN_ON: &str = "Press Enter to sign on to Service Tools.";

    /// One row: the label in its column, then the value.
    pub fn row(label: &str, value: &str) -> String {
        format!("{label:<14} {value}")
    }

    /// A service's row: its name in its column, then its state.
    pub fn service(name: &str, state: &str) -> String {
        row(SERVICE, &format!("{name:<10} {state}"))
    }

    use supervise::control::HostStatus;

    /// The rows of the screen: the label and the fact it shows.
    const ROWS: [(&str, &str); 5] = [
        (SLOT, super::fact::SLOT),
        (HOST, super::fact::HOST),
        (MANAGEMENT, super::fact::MANAGEMENT),
        (DATE, super::fact::DATE),
        (REFERENCE, super::fact::REFERENCE),
    ];

    /// The screen for `status`, under `banner`: the facts, a line a
    /// service, and how the console is reached.
    pub fn lines(banner: &str, status: &HostStatus) -> Vec<String> {
        let mut out = vec![banner.to_string()];
        for (label, key) in ROWS {
            let mut value = status.fact(key).unwrap_or("").to_string();
            if key == super::fact::SLOT
                && let Some(record) = status.fact(super::fact::RECORD)
            {
                value = format!("{value} ({record})");
            }
            out.push(row(label, &value));
        }
        for one in &status.services {
            let failures = match one.failures {
                0 => String::new(),
                1 => ", 1 failure".to_string(),
                n => format!(", {n} failures"),
            };
            out.push(service(&one.name, &format!("{}{failures}", one.state)));
        }
        out.push(SIGN_ON.to_string());
        out
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
    use supervise::control::{HostStatus, ServiceStatus};

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

    #[test]
    fn the_screen_is_the_banner_five_facts_and_a_line_a_service() {
        let fact = |k: &str, v: &str| (k.to_string(), v.to_string());
        let status = HostStatus {
            facts: vec![
                fact(fact::SLOT, "A"),
                fact(fact::RECORD, "default A, nothing pending"),
                fact(fact::HOST, "QSVS123456"),
                fact(fact::MANAGEMENT, "10.0.2.15/24 on eth0, gateway 10.0.2.2"),
                fact(fact::DATE, "2026-11-12 10:22 UTC"),
                fact(fact::REFERENCE, "C2001090 Base ready"),
                fact(fact::TIME, "1794478923"),
            ],
            services: vec![
                ServiceStatus {
                    name: "logd".into(),
                    state: "ready".into(),
                    pid: 2,
                    failures: 0,
                },
                ServiceStatus {
                    name: "netd".into(),
                    state: "restarting".into(),
                    pid: 0,
                    failures: 2,
                },
            ],
        };
        assert_eq!(
            screen::lines("QS/VS V1R0M0 abc1234", &status),
            [
                "QS/VS V1R0M0 abc1234",
                "Slot           A (default A, nothing pending)",
                "Host name      QSVS123456",
                "Management     10.0.2.15/24 on eth0, gateway 10.0.2.2",
                "Date and time  2026-11-12 10:22 UTC",
                "Reference      C2001090 Base ready",
                "Service        logd       ready",
                "Service        netd       restarting, 2 failures",
                "Press Enter to sign on to Service Tools.",
            ]
        );
    }
}
