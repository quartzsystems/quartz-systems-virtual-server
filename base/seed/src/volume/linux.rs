// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Finding the volume among the block devices, and reading the seed
//! file from it through a read-only mount.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};

use super::{SECTOR, fat_label};
use crate::format::{FILE, MAX_BYTES};

/// Block devices that are never a seed volume.
const SKIPPED: [&str; 4] = ["loop", "ram", "zram", "dm-"];

/// The first block device, in name order, that is a FAT volume
/// labelled `label`.
pub fn find(label: &str) -> Option<PathBuf> {
    let mut names: Vec<String> = fs::read_dir("/sys/class/block")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !SKIPPED.iter().any(|s| n.starts_with(s)))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| Path::new("/dev").join(n))
        .find(|device| {
            let mut sector = [0u8; SECTOR];
            File::open(device)
                .and_then(|mut f| f.read_exact(&mut sector))
                .is_ok_and(|()| fat_label(&sector).as_deref() == Some(label))
        })
}

/// The text of the seed file on `device`, mounted at `mount_at` for
/// the read.
pub fn read(device: &Path, mount_at: &Path) -> Result<String, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", device.display());
    fs::create_dir_all(mount_at).map_err(|e| at(&e))?;
    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC;
    mount(device, mount_at, "vfat", flags, None).map_err(|e| at(&e))?;
    let text = (|| {
        let path = mount_at.join(FILE);
        let size = fs::metadata(&path)
            .map_err(|e| format!("{FILE}: {e}"))?
            .len();
        if size > MAX_BYTES {
            return Err(format!("{FILE}: {size} bytes is more than {MAX_BYTES}"));
        }
        fs::read_to_string(&path).map_err(|e| format!("{FILE}: {e}"))
    })();
    let _ = unmount(mount_at, UnmountFlags::empty());
    let _ = fs::remove_dir(mount_at);
    text.map_err(|e| at(&e))
}
