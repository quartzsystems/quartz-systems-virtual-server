// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Test fixtures: a sysfs and procfs tree under the temporary directory,
//! and hand-built SMBIOS structures.

use std::fs;
use std::path::PathBuf;

use crate::Roots;

/// A fixture tree, removed when dropped. `sys/` and `proc/` are the roots.
pub(crate) struct Tree {
    root: PathBuf,
}

impl Tree {
    /// A new, empty tree; `name` is unique per test.
    pub(crate) fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("hostinv-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sys")).unwrap();
        fs::create_dir_all(root.join("proc")).unwrap();
        Tree { root }
    }

    /// The roots of the tree.
    pub(crate) fn roots(&self) -> Roots {
        Roots {
            sys: self.root.join("sys"),
            proc: self.root.join("proc"),
        }
    }

    /// The sysfs root.
    pub(crate) fn sys(&self) -> PathBuf {
        self.root.join("sys")
    }

    /// Make a directory and its parents.
    pub(crate) fn dir(&self, rel: &str) {
        fs::create_dir_all(self.root.join(rel)).unwrap();
    }

    /// Write a file, making its parents.
    pub(crate) fn file(&self, rel: &str, content: impl AsRef<[u8]>) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Make `rel` a symbolic link to `target`; the target may be absent.
    #[cfg(unix)]
    pub(crate) fn link(&self, rel: &str, target: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(self.root.join(target), path).unwrap();
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One SMBIOS structure: the header, `body` (the formatted area after
/// the header) and the string set.
pub(crate) fn structure(kind: u8, body: &[u8], strings: &[&str]) -> Vec<u8> {
    let length = u8::try_from(4 + body.len()).unwrap();
    let mut out = vec![kind, length, kind, 0];
    out.extend_from_slice(body);
    for s in strings {
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }
    if strings.is_empty() {
        out.push(0);
    }
    out.push(0);
    out
}

/// A type 9 structure with an address: designation, segment, bus,
/// device and function.
pub(crate) fn slot(designation: &str, segment: u16, bus: u8, device: u8, function: u8) -> Vec<u8> {
    let seg = segment.to_le_bytes();
    let devfn = (device << 3) | function;
    // Designation, type, width, usage, length, ID, characteristics.
    let body = [
        1, 0xa5, 0x0b, 0x04, 0x04, 1, 0, 0x0c, 0x01, seg[0], seg[1], bus, devfn,
    ];
    structure(9, &body, &[designation])
}
