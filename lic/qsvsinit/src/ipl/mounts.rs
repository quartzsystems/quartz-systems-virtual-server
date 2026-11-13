// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What PID 1 mounts: the kernel's file systems first, then cgroup2,
//! then `QSVSDATA` at `/data`.

use std::ffi::CStr;
use std::path::Path;

use rustix::mount::{MountFlags, UnmountFlags, mount, mount_remount, unmount};

/// One mount: source, mount point, type, flags, options.
struct Mount {
    source: &'static str,
    at: &'static str,
    fs: &'static str,
    flags: MountFlags,
    data: Option<&'static CStr>,
}

fn plain() -> MountFlags {
    MountFlags::NOSUID.union(MountFlags::NOEXEC)
}

fn nodev() -> MountFlags {
    plain().union(MountFlags::NODEV)
}

use loadsource::boot::EFIVARS;

fn kernel_table() -> [Mount; 5] {
    [
        Mount {
            source: "proc",
            at: "/proc",
            fs: "proc",
            flags: nodev(),
            data: None,
        },
        Mount {
            source: "sysfs",
            at: "/sys",
            fs: "sysfs",
            flags: nodev(),
            data: None,
        },
        Mount {
            source: "devtmpfs",
            at: "/dev",
            fs: "devtmpfs",
            flags: plain(),
            data: Some(c"mode=0755"),
        },
        Mount {
            source: "tmpfs",
            at: "/run",
            fs: "tmpfs",
            flags: nodev(),
            data: Some(c"mode=0755"),
        },
        // The firmware boot entries the slot is chosen by.
        Mount {
            source: "efivarfs",
            at: EFIVARS,
            fs: "efivarfs",
            flags: nodev(),
            data: None,
        },
    ]
}

fn mount_all(table: &[Mount]) -> Vec<(&'static str, String)> {
    let mut failed = Vec::new();
    for m in table {
        let _ = std::fs::create_dir_all(m.at);
        if let Err(e) = mount(m.source, m.at, m.fs, m.flags, m.data) {
            failed.push((m.at, e.to_string()));
        }
    }
    failed
}

/// Mount the kernel's file systems, in order. Returns what failed and
/// why: the console is not open yet, so the caller reports.
pub fn kernel() -> Vec<(&'static str, String)> {
    mount_all(&kernel_table())
}

/// Mount cgroup2.
pub fn cgroups() -> Vec<(&'static str, String)> {
    mount_all(&[Mount {
        source: "cgroup2",
        at: "/sys/fs/cgroup",
        fs: "cgroup2",
        flags: nodev(),
        data: None,
    }])
}

/// The control group of PID 1 and of everything it starts:
/// `qsvs/host`. PID 1 joins it here; the partition manager later
/// keeps it to the cores reserved for the host.
pub fn host_group() -> Result<(), String> {
    let dir = Path::new("/sys/fs/cgroup").join(resource::cgroup::HOST);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("cgroup.procs"), "1").map_err(|e| format!("{}: {e}", dir.display()))
}

/// Mount the xfs on `device` at `at`.
pub fn data(device: &Path, at: &str) -> Result<(), String> {
    let _ = std::fs::create_dir_all(at);
    mount(device, at, "xfs", nodev(), None).map_err(|e| e.to_string())
}

/// Unmount `at`. When something still holds it, it is at least made
/// read-only, which also leaves its log clean.
pub fn unmount_data(at: &str) -> Result<(), String> {
    match unmount(at, UnmountFlags::empty()) {
        Ok(()) => Ok(()),
        Err(busy) => {
            let read_only = nodev().union(MountFlags::RDONLY);
            match mount_remount(at, read_only, c"") {
                Ok(()) => Err(format!("{busy}; made read-only instead")),
                Err(e) => Err(format!("{busy}; not made read-only either: {e}")),
            }
        }
    }
}
