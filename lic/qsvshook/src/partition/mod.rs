// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition hook: one partition, started by hand. There is no
//! partition manager before Phase 4, so a test image starts `qsvmm`
//! itself, for the profile it finds on a volume labelled
//! [`VOLUME_LABEL`], and does what the partition manager will: posts
//! the reference codes `qsvmm` reports, relays the partition's serial
//! output to the host's serial port, each line after the partition's
//! name, and posts how the partition ended. A release image has
//! `qsvmm` in it and nothing that starts it.
//!
//! - [`runner`]: the service, `qsvshook partition`.
//! - [`probe`]: requests no guest kernel sends, made of a real host
//!   device through the virtio-scsi device itself.

#[cfg(target_os = "linux")]
pub mod probe;
#[cfg(target_os = "linux")]
pub mod runner;

/// The label of the FAT volume the profile is on.
pub const VOLUME_LABEL: &str = "QSVSPART";
/// The profile's name on that volume.
pub const PROFILE_FILE: &str = "partition.toml";
/// Where the profile is kept on the host: the channel's `put` writes a
/// new one here before a start.
pub const PROFILE: &str = "/run/partition.toml";
/// `qsvmm`'s process id while it runs.
pub const PID_FILE: &str = "/run/partition.pid";
/// How the last run ended: `exit <code>` or `signal <number>`.
pub const END_FILE: &str = "/run/partition.end";
/// Made by the channel to have the partition started again.
pub const START_FILE: &str = "/run/partition.start";
/// The VMM.
pub const VMM: &str = "/qsvs/bin/qsvmm";

/// What comes between the partition's name and a line of its serial
/// output on the host's serial port.
pub const SERIAL_MARK: &str = "| ";
/// The same for a line `qsvmm` itself wrote that is not a reference code.
pub const VMM_MARK: &str = "! ";

/// A line of the partition's serial output, as relayed.
pub fn relayed(name: &str, line: &str) -> String {
    format!("{name}{SERIAL_MARK}{line}")
}

/// A line of `qsvmm`'s own, as relayed.
pub fn reported(name: &str, line: &str) -> String {
    format!("{name}{VMM_MARK}{line}")
}

/// A line of `qsvmm`'s standard error as a reference code and its text.
pub fn code_of(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix(refcode::WORD)?.strip_prefix(' ')?;
    let (code, text) = rest.split_once(' ')?;
    refcode::find(code).map(|_| (code, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_line_with_a_code_of_the_table_is_a_code() {
        assert_eq!(
            code_of("REF C2005090 Partition started: G1 side A"),
            Some(("C2005090", "Partition started: G1 side A"))
        );
        assert_eq!(code_of("REF C1001000 IPL started"), None);
        assert_eq!(code_of("qsvmm: unit 0 reads 1"), None);
        assert_eq!(code_of("REF C2005090"), None);
        assert_eq!(
            relayed("G1", "SRC C6001090  IPL complete"),
            "G1| SRC C6001090  IPL complete"
        );
        assert_eq!(reported("G1", "qsvmm: G1 ended"), "G1! qsvmm: G1 ended");
    }
}
