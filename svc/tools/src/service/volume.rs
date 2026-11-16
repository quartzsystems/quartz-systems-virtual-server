// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A removable volume of the host, for a host no management console
//! manages: mounted by its kernel name (`sdb1`) to read an update image
//! from or to copy dumps to, and unmounted when the panel is done.

use std::fs;
use std::path::{Path, PathBuf};

/// Where a volume is mounted.
const AT: &str = "/run/qsvstools/volume";

/// The file systems a volume is tried as.
const KINDS: [&str; 5] = ["vfat", "exfat", "ext4", "xfs", "iso9660"];

/// A volume mounted; unmounted when dropped.
#[derive(Debug)]
pub struct Mounted {
    at: PathBuf,
}

impl Mounted {
    /// Where it is mounted.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.at
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        rustix::fs::sync();
        let _ = rustix::mount::unmount(&self.at, rustix::mount::UnmountFlags::empty());
    }
}

/// Whether `device` is a kernel name of a disk or a partition and
/// nothing else: letters and digits.
#[must_use]
pub fn valid_device(device: &str) -> bool {
    !device.is_empty()
        && device.len() <= 32
        && device
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Mount `/dev/<device>`, to write when `write`.
///
/// # Errors
/// It is no device's name, it is a load-source disk, or it holds no
/// volume that mounts.
pub fn mount(device: &str, write: bool) -> Result<Mounted, String> {
    if !valid_device(device) {
        return Err(format!("{device} is not a device's name: type it as sdb1."));
    }
    // The host's own disks are never a removable volume.
    let disk = loadsource::disks::disk_of(&Path::new("/dev").join(device))
        .unwrap_or_else(|| device.to_string());
    if loadsource::disks::load_sources(&release::LOAD_SOURCE)
        .iter()
        .any(|s| s.disk == disk)
    {
        return Err(format!("{device} is a load-source disk of this host."));
    }
    let node = Path::new("/dev").join(device);
    let at = PathBuf::from(AT);
    fs::create_dir_all(&at).map_err(|e| format!("{AT}: {e}"))?;
    let _ = rustix::mount::unmount(&at, rustix::mount::UnmountFlags::empty());
    let mut flags = rustix::mount::MountFlags::NODEV
        | rustix::mount::MountFlags::NOSUID
        | rustix::mount::MountFlags::NOEXEC;
    if !write {
        flags |= rustix::mount::MountFlags::RDONLY;
    }
    let mounted = KINDS
        .iter()
        .filter(|k| !write || **k != "iso9660")
        .any(|k| rustix::mount::mount(&node, &at, *k, flags, None).is_ok());
    if mounted {
        Ok(Mounted { at })
    } else {
        Err(format!(
            "{device} holds no volume that can be {}.",
            if write { "written" } else { "read" }
        ))
    }
}

/// The files under `root` (two levels at most) whose names end in
/// `ext`, as paths within the volume, sorted.
#[must_use]
pub fn files(root: &Path, ext: &str) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, depth: usize, ext: &str, out: &mut Vec<String>) {
        let Ok(read) = fs::read_dir(dir) else { return };
        for e in read.flatten() {
            let path = e.path();
            if path.is_dir() && depth > 0 {
                walk(root, &path, depth - 1, ext, out);
            } else if path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case(ext))
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(rel.display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, 1, ext, &mut out);
    out.sort();
    out
}

/// The file `rel` within the volume at `root`; none when the path
/// leaves it.
#[must_use]
pub fn within(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    (!rel.split('/').any(|p| p == ".." || p.is_empty())).then(|| root.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_is_a_kernel_name_and_a_path_stays_within() {
        assert!(valid_device("sdb1") && valid_device("nvme1n1p1"));
        for bad in ["", "../sda", "sdb 1", "SDB1", "/dev/sdb"] {
            assert!(!valid_device(bad), "{bad}");
        }
        let root = Path::new("/run/v");
        assert_eq!(
            within(root, "a/b.upd"),
            Some(PathBuf::from("/run/v/a/b.upd"))
        );
        assert_eq!(within(root, "../etc/x"), None);
        let d = std::env::temp_dir().join(format!("tools-volume-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("updates")).unwrap();
        fs::write(d.join("QSVS-V1R0M1.UPD"), "x").unwrap();
        fs::write(d.join("updates/b.upd"), "x").unwrap();
        fs::write(d.join("notes.txt"), "x").unwrap();
        assert_eq!(files(&d, "upd"), ["QSVS-V1R0M1.UPD", "updates/b.upd"]);
        fs::remove_dir_all(&d).unwrap();
    }
}
