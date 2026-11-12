// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What QS/VS asks of its kernel: the fragment, the baked command line
//! and the symbols the final `.config` is checked for, group by group as
//! `kernel/config/qsvs.config` has them. A merge that drops one fails
//! the build.

/// The kernel configuration fragment in `kernel/config/`.
pub const FRAGMENT: &str = "qsvs.config";

/// The baked command line, before each UKI's own word.
pub const CMDLINE: &str = "console=ttyS0,115200 console=tty0 panic=-1 quiet";

/// Symbols QS/VS needs `=y`, past the pipeline's baseline.
pub const REQUIRED_Y: &[&str] = &[
    // Hypervisor.
    "VIRTUALIZATION",
    "KVM",
    "KVM_INTEL",
    "KVM_AMD",
    "EVENTFD",
    "IRQ_BYPASS_MANAGER",
    "VHOST_NET",
    "TUN",
    // Networking core.
    "NET",
    "INET",
    "IPV6",
    "UNIX",
    "PACKET",
    // Virtual Ethernet.
    "BRIDGE",
    "BRIDGE_VLAN_FILTERING",
    "VLAN_8021Q",
    "BONDING",
    // Resource control.
    "CGROUPS",
    "CPUSETS",
    "CGROUP_SCHED",
    "FAIR_GROUP_SCHED",
    "CFS_BANDWIDTH",
    "MEMCG",
    "CGROUP_PIDS",
    "HUGETLBFS",
    "HUGETLB_PAGE",
    "SECCOMP",
    "SECCOMP_FILTER",
    // Storage.
    "XFS_FS",
    "BLK_DEV_UBLK",
    "IO_URING",
    "BLK_DEV_NVME",
    "SATA_AHCI",
    "SCSI",
    "BLK_DEV_SD",
    "MEGARAID_SAS",
    "SCSI_MPT3SAS",
    "SCSI_SMARTPQI",
    "SCSI_HPSA",
    "SCSI_AACRAID",
    "SCSI_MPI3MR",
    "CHR_DEV_ST",
    "CHR_DEV_SG",
    "SCSI_FC_ATTRS",
    "SCSI_QLA_FC",
    // The ESP and the firmware boot entries.
    "VFAT_FS",
    "NLS_CODEPAGE_437",
    "NLS_ISO8859_1",
    "EFIVAR_FS",
    // Physical NICs.
    "E1000",
    "E1000E",
    "IGB",
    "IGC",
    "IXGBE",
    "I40E",
    "ICE",
    "TIGON3",
    "BNXT",
    "MLX5_CORE",
    "MLX5_CORE_EN",
    // Pass-through.
    "VFIO",
    "VFIO_PCI",
    "INTEL_IOMMU",
    "AMD_IOMMU",
    "IRQ_REMAP",
    // Under QEMU.
    "VIRTIO_PCI",
    "VIRTIO_BLK",
    "SCSI_VIRTIO",
    "VIRTIO_NET",
    "VIRTIO_CONSOLE",
    "HW_RANDOM",
    "HW_RANDOM_VIRTIO",
    // The Service Tools console.
    "VT",
    "VT_CONSOLE",
    "FB",
    "FRAMEBUFFER_CONSOLE",
    "DRM_SIMPLEDRM",
    "DRM_FBDEV_EMULATION",
    "SERIAL_8250",
    "SERIAL_8250_CONSOLE",
    "INPUT_EVDEV",
    "KEYBOARD_ATKBD",
    "HID",
    "USB_HID",
    // The base system: the RTC, the power button, DMI, signalfd.
    "RTC_CLASS",
    "RTC_DRV_CMOS",
    "RTC_INTF_SYSFS",
    "RTC_INTF_DEV",
    "ACPI_BUTTON",
    "DMI",
    "DMIID",
    "SIGNALFD",
    // The installer's media: ISO 9660 on a CD-ROM or a USB stick.
    "ISO9660_FS",
    "BLK_DEV_SR",
    "USB_STORAGE",
    "USB_UAS",
];

/// Symbols QS/VS must not have `=y`, past the baseline.
pub const REQUIRED_N: &[&str] = &[
    // Disks keep their order from boot to boot.
    "SCSI_SCAN_ASYNC",
    // The image's bytes do not depend on where it was built.
    "IKCONFIG",
    // Nothing but the QSI server listens; no packet filter.
    "NETFILTER",
    "EXT4_FS",
    "WIRELESS",
    "WLAN",
    "CFG80211",
    "BT",
    "SOUND",
    "SND",
    "MEDIA_SUPPORT",
    // PID 1 sets the clock from the RTC; the kernel does not.
    "RTC_HCTOSYS",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The symbols Phase 0 names (the handoff's Section 2), each of
    /// which the independent `.config` check must cover.
    const PHASE0: &[&str] = &[
        "KVM",
        "KVM_INTEL",
        "KVM_AMD",
        "VHOST_NET",
        "TUN",
        "EVENTFD",
        "IRQ_BYPASS_MANAGER",
        "BRIDGE",
        "BRIDGE_VLAN_FILTERING",
        "VLAN_8021Q",
        "BONDING",
        "CGROUPS",
        "CPUSETS",
        "CGROUP_SCHED",
        "FAIR_GROUP_SCHED",
        "CFS_BANDWIDTH",
        "MEMCG",
        "CGROUP_PIDS",
        "HUGETLBFS",
        "HUGETLB_PAGE",
        "SECCOMP",
        "SECCOMP_FILTER",
        "XFS_FS",
        "BLK_DEV_UBLK",
        "IO_URING",
        "BLK_DEV_NVME",
        "SATA_AHCI",
        "SCSI",
        "BLK_DEV_SD",
        "MEGARAID_SAS",
        "SCSI_MPT3SAS",
        "CHR_DEV_ST",
        "CHR_DEV_SG",
        "SCSI_FC_ATTRS",
        "SCSI_QLA_FC",
        "E1000E",
        "IGB",
        "IXGBE",
        "I40E",
        "ICE",
        "TIGON3",
        "BNXT",
        "MLX5_CORE",
        "MLX5_CORE_EN",
        "VFIO",
        "VFIO_PCI",
        "INTEL_IOMMU",
        "AMD_IOMMU",
        "IRQ_REMAP",
        "VIRTIO_PCI",
        "VIRTIO_BLK",
        "SCSI_VIRTIO",
        "VIRTIO_NET",
        "VIRTIO_CONSOLE",
        "HW_RANDOM_VIRTIO",
    ];

    /// What Phase 1 needs and its handoff names (docs/PHASE1.md has the
    /// reason for each). `INPUT_EVDEV`, `VLAN_8021Q`, `XFS_FS`, `VFAT_FS`
    /// and `EFIVAR_FS` were Phase 0's already.
    const PHASE1: &[&str] = &[
        "RTC_CLASS",
        "RTC_DRV_CMOS",
        "RTC_INTF_SYSFS",
        "RTC_INTF_DEV",
        "ACPI_BUTTON",
        "INPUT_EVDEV",
        "DMI",
        "DMIID",
        "SIGNALFD",
        "VLAN_8021Q",
        "XFS_FS",
        "VFAT_FS",
        "EFIVAR_FS",
        "UNIX",
    ];

    /// What Phase 2 needs (docs/PHASE2.md has the reason for each).
    /// The framebuffer, evdev, ublk and `io_uring` were Phase 0's already.
    const PHASE2: &[&str] = &[
        "ISO9660_FS",
        "BLK_DEV_SR",
        "USB_STORAGE",
        "USB_UAS",
        "BLK_DEV_UBLK",
        "IO_URING",
        "FB",
        "DRM_SIMPLEDRM",
        "DRM_FBDEV_EMULATION",
        "INPUT_EVDEV",
        "KEYBOARD_ATKBD",
        "USB_HID",
    ];

    #[test]
    fn every_phase_2_symbol_is_checked() {
        for sym in PHASE2 {
            assert!(REQUIRED_Y.contains(sym), "{sym} is not in REQUIRED_Y");
        }
    }

    #[test]
    fn every_phase_1_symbol_is_checked() {
        for sym in PHASE1 {
            assert!(REQUIRED_Y.contains(sym), "{sym} is not in REQUIRED_Y");
        }
        assert!(REQUIRED_N.contains(&"RTC_HCTOSYS"));
    }

    #[test]
    fn every_phase_0_symbol_is_checked() {
        for sym in PHASE0 {
            assert!(REQUIRED_Y.contains(sym), "{sym} is not in REQUIRED_Y");
        }
    }

    #[test]
    fn no_symbol_is_listed_twice_or_both_ways() {
        for (i, sym) in REQUIRED_Y.iter().enumerate() {
            assert!(!REQUIRED_Y[..i].contains(sym), "{sym} twice");
            assert!(!REQUIRED_N.contains(sym), "{sym} both ways");
        }
    }

    /// The fragment in the tree asks for every symbol the check wants,
    /// but for the ones other symbols select.
    #[test]
    fn the_fragment_sets_what_the_check_requires() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../kernel/config/qsvs.config"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let selected = ["IRQ_BYPASS_MANAGER"];
        for sym in REQUIRED_Y {
            let set = text.lines().any(|l| l == format!("CONFIG_{sym}=y"));
            assert!(set || selected.contains(sym), "{sym} is not in {FRAGMENT}");
        }
        for sym in [
            "SCSI_SCAN_ASYNC",
            "IKCONFIG",
            "NETFILTER",
            "EXT4_FS",
            "RTC_HCTOSYS",
        ] {
            let unset = format!("# CONFIG_{sym} is not set");
            assert!(text.lines().any(|l| l == unset), "{sym}");
        }
    }
}
