// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Taking a host dump (`docs/DUMPS.md`): `base/servicedump`'s service
//! data with what is a host's own beside it: each partition, pool and
//! switch as its service says it, and the control group tree. The
//! consoles' pins are left out with the base's secrets.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use servicedump::{Info, Store};

pub use servicedump::Asked;

/// Every file never taken, wherever it is, and why: the base's and the
/// host's own. A test holds every file that keeps a secret to it.
pub const LEFT_OUT: &[(&str, &str)] = &[
    ("identity.key", "the host's private key"),
    (
        "users.db",
        "the service tools users and their password hashes",
    ),
    (
        "qsi-consoles.toml",
        "the management consoles' pins: pairing material",
    ),
];

/// The host's own secrets, beside the base's.
pub const OWN_LEFT_OUT: &[(&str, &str)] = &[(
    "qsi-consoles.toml",
    "the management consoles' pins: pairing material",
)];

/// Whether a file named `name` holding `data` keeps a secret.
#[must_use]
pub fn secret(name: &str, data: &[u8]) -> bool {
    servicedump::take::secret(name, data, OWN_LEFT_OUT)
}

/// Where the host keeps what a dump takes.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The log service's directory: the system log, the reference code
    /// history, each boot's kernel log.
    pub log: PathBuf,
    /// The state files.
    pub state: PathBuf,
    /// The problem log's directory.
    pub problems: PathBuf,
    /// The control group tree's root.
    pub cgroup: PathBuf,
    /// The kernel's process file system.
    pub proc: PathBuf,
}

/// What the services say: each a text, or why it could not be had.
pub trait Sources {
    /// The partitions: each status, definition and profile.
    fn partitions(&mut self) -> String;
    /// The storage pools, their virtual disks and the media library.
    fn pools(&mut self) -> String;
    /// The virtual switches and their ports.
    fn switches(&mut self) -> String;
}

/// The control group tree: each group, what it holds and its limits.
#[must_use]
pub fn cgroups(root: &Path) -> String {
    const FILES: &[&str] = &[
        "cgroup.controllers",
        "cgroup.procs",
        "cpu.max",
        "cpu.weight",
        "cpuset.cpus",
        "cpuset.cpus.effective",
        "cpuset.mems",
        "memory.max",
        "memory.current",
        "hugetlb.2MB.max",
        "hugetlb.2MB.current",
    ];
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut String) {
        let rel = dir.strip_prefix(root).unwrap_or(dir).display().to_string();
        let _ = writeln!(out, "[/{rel}]");
        for f in FILES {
            if let Ok(v) = fs::read_to_string(dir.join(f)) {
                let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
                let _ = writeln!(out, "{f} = {v}");
            }
        }
        out.push('\n');
        if depth == 0 {
            return;
        }
        let Ok(read) = fs::read_dir(dir) else { return };
        let mut subs: Vec<PathBuf> = read
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .collect();
        subs.sort();
        for s in subs {
            walk(root, &s, depth - 1, out);
        }
    }
    let mut out = String::new();
    walk(root, root, 4, &mut out);
    out
}

/// The host's own entries, in the order a dump has always had them.
struct Own<'a> {
    sources: &'a mut dyn Sources,
    cgroup: &'a Path,
}

impl servicedump::Sources for Own<'_> {
    fn entries(&mut self) -> Vec<(String, Vec<u8>)> {
        vec![
            (
                "services/partitions.txt".into(),
                self.sources.partitions().into_bytes(),
            ),
            (
                "services/pools.txt".into(),
                self.sources.pools().into_bytes(),
            ),
            (
                "services/switches.txt".into(),
                self.sources.switches().into_bytes(),
            ),
            ("host/cgroups.txt".into(), cgroups(self.cgroup).into_bytes()),
        ]
    }
}

/// Take a dump into `store`, at `now`: its name and size, or why it
/// could not be written (nothing of it is kept then). The oldest dumps
/// are put out first to make room.
///
/// # Errors
/// The archive could not be written whole.
pub fn take(
    store: &Store,
    paths: &Paths,
    sources: &mut dyn Sources,
    asked: &Asked,
    now: i64,
) -> Result<(Info, Vec<String>), String> {
    let base = servicedump::Paths {
        log: paths.log.clone(),
        state: paths.state.clone(),
        problems: paths.problems.clone(),
        proc: paths.proc.clone(),
    };
    let mut own = Own {
        sources,
        cgroup: &paths.cgroup,
    };
    servicedump::take(store, &base, &mut own, asked, now)
}
