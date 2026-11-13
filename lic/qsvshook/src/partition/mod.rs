// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition hooks. A test image's partitions are the partition
//! manager's, as a release image's are: the hooks carry the harness's
//! requests to its socket and bring back the answers (the channel's
//! `partd` command), and relay what the partitions say to the host's
//! serial port, where the harness reads. A release image has no such
//! path: nothing in it sends the partition manager a request to act.
//!
//! - [`relay`]: the service, `qsvshook partition`.
//! - [`probe`]: requests no guest kernel sends, made of a real host
//!   device through the virtio-scsi device itself.

#[cfg(target_os = "linux")]
pub mod probe;
#[cfg(target_os = "linux")]
pub mod relay;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relayed_line_follows_its_partition_s_name() {
        assert_eq!(
            relayed("G1", "SRC C6001090  IPL complete"),
            "G1| SRC C6001090  IPL complete"
        );
        assert_eq!(reported("G1", "qsvmm: G1 ended"), "G1! qsvmm: G1 ended");
    }
}
