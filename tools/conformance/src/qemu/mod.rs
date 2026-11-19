// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The QEMU driver of `base/qemu` as QS/VS's harness uses it: its
//! variables, its disks' serial numbers and the places of the devices a
//! host gives away; the test hooks' channel, the golden screens and the
//! load source on an image are the harness's own.

pub mod channel;
pub mod golden;
pub mod image;
pub mod session;

use std::env;
use std::path::PathBuf;

pub use driver::launch::{FIRST_UPLINK_PLACE, kvm_usable};
pub use driver::{Drive, Far, Launch, Passed, UplinkNic, args, serial, term, vars, wire};

use crate::error::HarnessError;

/// Environment override for the QEMU binary path.
pub const QEMU_ENV: &str = "QSVS_QEMU";
/// Set to run without KVM even where `/dev/kvm` is usable.
pub const NO_KVM_ENV: &str = "QSVS_NO_KVM";
/// Turns the emulated IOMMU off for every host but a suite's own.
pub const NO_IOMMU_ENV: &str = "QSVS_NO_IOMMU";
/// The serial number of the boot disk.
pub const DISK_SERIAL: &str = "QSVSHD0";
/// The serial number of the `NVMe` function beside a shared disk.
pub const BESIDE_SERIAL: &str = "QSVSBESIDE";
/// The serial numbers of the pair's two functions.
pub const PAIR_SERIALS: [&str; 2] = ["QSVSPAIR0", "QSVSPAIR1"];

/// The places of the devices to give away: after the uplinks.
pub const PASSED_NIC: u8 = 0x1A;
/// The `NVMe` controller's place.
pub const PASSED_NVME: u8 = 0x1B;
/// The pair's place.
pub const PASSED_PAIR: u8 = 0x1C;
/// The place of a disk and an adapter that share a group.
pub const PASSED_SHARED: u8 = 0x1D;

/// Find the system emulator: `QSVS_QEMU`, else `PATH`.
pub fn locate() -> Result<PathBuf, HarnessError> {
    Ok(driver::launch::locate(QEMU_ENV)?)
}

/// KVM is used when it is usable and `QSVS_NO_KVM` is unset.
pub fn kvm_wanted() -> bool {
    driver::launch::kvm_wanted(NO_KVM_ENV)
}

/// Every host has the emulated IOMMU with interrupt remapping, as a
/// machine that gives devices to partitions has (Phase 10), unless
/// `QSVS_NO_IOMMU` is set: then only a suite that asks for it.
pub fn iommu_wanted() -> bool {
    env::var_os(NO_IOMMU_ENV).is_none()
}

/// The forwarding of a harness port to the QSI host server's.
pub fn qsi_forward(port: u16) -> Vec<(u16, u16)> {
    vec![(port, release::QSI_PORT)]
}

/// The harness port forwarded to the QSI host server's; 0 for none.
pub fn qsi_port(launch: &Launch) -> u16 {
    launch
        .forwards
        .iter()
        .find(|(_, guest)| *guest == release::QSI_PORT)
        .map_or(0, |(host, _)| *host)
}

/// OVMF, by QS/VS's variables.
pub mod firmware {
    use std::path::{Path, PathBuf};

    use crate::error::HarnessError;

    /// Environment variable naming the OVMF code image.
    pub const CODE_ENV: &str = "QSVS_OVMF_CODE";
    /// Environment variable naming OVMF's variable store template.
    pub const VARS_ENV: &str = "QSVS_OVMF_VARS";

    /// The code image: `QSVS_OVMF_CODE`, else the distribution defaults.
    pub fn locate() -> Result<PathBuf, HarnessError> {
        Ok(driver::firmware::locate(CODE_ENV)?)
    }

    /// The variable store template that goes with `code`.
    pub fn locate_vars(code: &Path) -> Result<PathBuf, HarnessError> {
        Ok(driver::firmware::locate_vars(code, VARS_ENV)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_harness_s_names_reach_the_command_line() {
        let launch = Launch {
            qemu: "/usr/bin/qemu-system-x86_64".into(),
            firmware: "/fw/OVMF_CODE_4M.fd".into(),
            disk: Some("/t/disk.img".into()),
            disk_serial: DISK_SERIAL.into(),
            nic: Some("52:54:00:12:34:56".into()),
            forwards: qsi_forward(40000),
            memory_mb: 1024,
            cpus: 1,
            ..Launch::default()
        };
        let words: Vec<String> = args(&launch)
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert!(words.contains(&"virtio-blk-pci,drive=hd,serial=QSVSHD0".into()));
        assert!(words.contains(&"user,id=net0,hostfwd=tcp:127.0.0.1:40000-10.0.2.15:2300".into()));
        assert_eq!(qsi_port(&launch), 40000);
        assert_eq!(qsi_port(&Launch::default()), 0);
    }
}
