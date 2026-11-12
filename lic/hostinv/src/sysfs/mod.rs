// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Reading sysfs and procfs nodes. A node that cannot be read is `None`.

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// The text of a node, trimmed.
pub(crate) fn text(path: impl AsRef<Path>) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    Some(trimmed.to_string())
}

/// The number in a node.
pub(crate) fn number<T: FromStr>(path: impl AsRef<Path>) -> Option<T> {
    text(path)?.parse().ok()
}

/// A hex node such as `0x8086`: no prefix, lower case, `digits` wide.
/// Empty when the node is unreadable.
pub(crate) fn hex(path: impl AsRef<Path>, digits: usize) -> String {
    let Some(text) = text(path) else {
        return String::new();
    };
    let text = text.to_ascii_lowercase();
    let value = text.strip_prefix("0x").unwrap_or(&text);
    format!("{value:0>digits$}")
}

/// The file name of a symbolic link's target: the driver of a `driver`
/// link.
pub(crate) fn link_name(path: impl AsRef<Path>) -> Option<String> {
    let target = fs::read_link(path).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// The entries of a directory, sorted.
pub(crate) fn names(dir: impl AsRef<Path>) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The number of a name that is `prefix` and then digits only.
pub(crate) fn numbered(name: &str, prefix: &str) -> Option<u32> {
    let digits = name.strip_prefix(prefix)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// A node with its links resolved.
pub(crate) struct Node {
    /// The canonical path.
    pub(crate) path: PathBuf,
    /// The canonical sysfs root.
    pub(crate) sys: PathBuf,
    /// The path's components below the sysfs root; empty when the path
    /// does not resolve or is not below it.
    pub(crate) parts: Vec<String>,
}

/// Resolve `path`, an entry of a class directory, into `/sys/devices`.
pub(crate) fn resolve(sys: &Path, path: &Path) -> Node {
    let canonical = fs::canonicalize(path).ok();
    let root = fs::canonicalize(sys).ok();
    let parts = match (&canonical, &root) {
        (Some(canonical), Some(root)) => canonical
            .strip_prefix(root)
            .map(|rel| {
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    Node {
        path: canonical.unwrap_or_else(|| path.to_path_buf()),
        sys: root.unwrap_or_else(|| sys.to_path_buf()),
        parts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Tree;

    #[test]
    fn nodes_read_trimmed_and_missing_nodes_read_as_nothing() {
        let tree = Tree::new("sysfs-text");
        tree.file("sys/a", "  QEMU HARDDISK   \n");
        tree.file("sys/n", "42\n");
        tree.file("sys/v", "0x1AF4\n");
        tree.file("sys/short", "0x1\n");
        let sys = tree.sys();
        assert_eq!(text(sys.join("a")).as_deref(), Some("QEMU HARDDISK"));
        assert_eq!(text(sys.join("missing")), None);
        assert_eq!(number::<u64>(sys.join("n")), Some(42));
        assert_eq!(number::<u64>(sys.join("a")), None);
        assert_eq!(hex(sys.join("v"), 4), "1af4");
        assert_eq!(hex(sys.join("short"), 4), "0001");
        assert_eq!(hex(sys.join("missing"), 4), "");
        assert_eq!(names(&sys), ["a", "n", "short", "v"]);
        assert!(names(sys.join("missing")).is_empty());
    }

    #[test]
    fn a_numbered_name_is_a_prefix_and_digits() {
        assert_eq!(numbered("host12", "host"), Some(12));
        assert_eq!(numbered("host", "host"), None);
        assert_eq!(numbered("hostage", "host"), None);
        assert_eq!(numbered("nvme0n1", "nvme"), None);
        assert_eq!(numbered("ata3", "host"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_class_entry_resolves_to_its_device_path() {
        let tree = Tree::new("sysfs-resolve");
        tree.dir("sys/devices/pci0000:00/0000:00:03.0/virtio1/block/vda");
        tree.link(
            "sys/class/block/vda",
            "sys/devices/pci0000:00/0000:00:03.0/virtio1/block/vda",
        );
        tree.link("sys/class/block/gone", "sys/devices/none");
        let sys = tree.sys();
        let node = resolve(&sys, &sys.join("class/block/vda"));
        assert_eq!(
            node.parts,
            [
                "devices",
                "pci0000:00",
                "0000:00:03.0",
                "virtio1",
                "block",
                "vda"
            ]
        );
        assert_eq!(
            link_name(sys.join("class/block/vda")).as_deref(),
            Some("vda")
        );
        assert!(
            resolve(&sys, &sys.join("class/block/gone"))
                .parts
                .is_empty()
        );
    }
}
