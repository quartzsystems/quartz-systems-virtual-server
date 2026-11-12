// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The kernel file systems PID 1 mounts before anything else.

use std::ffi::CStr;

use rustix::mount::{MountFlags, mount};

/// One mount: source, mount point, type, flags, options.
struct Mount {
    source: &'static str,
    at: &'static str,
    fs: &'static str,
    flags: MountFlags,
    data: Option<&'static CStr>,
}

fn table() -> [Mount; 5] {
    let plain = MountFlags::NOSUID.union(MountFlags::NOEXEC);
    let nodev = plain.union(MountFlags::NODEV);
    [
        Mount {
            source: "proc",
            at: "/proc",
            fs: "proc",
            flags: nodev,
            data: None,
        },
        Mount {
            source: "sysfs",
            at: "/sys",
            fs: "sysfs",
            flags: nodev,
            data: None,
        },
        Mount {
            source: "devtmpfs",
            at: "/dev",
            fs: "devtmpfs",
            flags: plain,
            data: Some(c"mode=0755"),
        },
        Mount {
            source: "tmpfs",
            at: "/run",
            fs: "tmpfs",
            flags: nodev,
            data: Some(c"mode=0755"),
        },
        Mount {
            source: "cgroup2",
            at: "/sys/fs/cgroup",
            fs: "cgroup2",
            flags: nodev,
            data: None,
        },
    ]
}

/// Mount them all, in order. Returns what failed and why: the console
/// is not open yet, so the caller reports.
pub fn mount_all() -> Vec<(&'static str, String)> {
    let mut failed = Vec::new();
    for m in table() {
        let _ = std::fs::create_dir_all(m.at);
        if let Err(e) = mount(m.source, m.at, m.fs, m.flags, m.data) {
            failed.push((m.at, e.to_string()));
        }
    }
    failed
}
