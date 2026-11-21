// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Taking a service dump: service data, not memory. What every
//! appliance gathers (its logs, its problem log, every state file but
//! those that hold a secret, its services' status, the kernel's view of
//! the machine) and what the product adds of its own, in one archive
//! with a manifest of what is in it and of what was left out.

use std::fmt::Write as _;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::store::{BY, Info, Store, name_at};
use crate::zip::Writer;

/// The files of the base crates that keep a secret, never taken
/// wherever they are, and why: the identity's key (`base/identity`)
/// and the service tools users (`base/users`). A test holds them to
/// those crates' file names; the product adds its own
/// ([`crate::store::Kind::left_out`]).
pub const LEFT_OUT: &[(&str, &str)] = &[
    ("identity.key", "the host's private key"),
    (
        "users.db",
        "the service tools users and their password hashes",
    ),
];

/// Words that only a secret's file holds: a file with one is left out
/// whatever its name.
const SECRET_MARKS: &[&str] = &["PRIVATE KEY", "$argon2"];

/// The most taken of one file: its newest bytes.
pub const MAX_FILE: u64 = 64 * 1024 * 1024;
/// The most one dump may be.
pub const MAX_DUMP: u64 = 512 * 1024 * 1024;

/// Where an appliance keeps what every dump takes.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The log service's directory: the system log, the reference code
    /// history, each boot's kernel log.
    pub log: PathBuf,
    /// The state files.
    pub state: PathBuf,
    /// The problem log's directory.
    pub problems: PathBuf,
    /// The kernel's process file system.
    pub proc: PathBuf,
}

/// What the product adds of its own: each entry's name in the archive
/// and its bytes, in order.
pub trait Sources {
    /// The product's entries.
    fn entries(&mut self) -> Vec<(String, Vec<u8>)>;
}

/// What a dump is asked with.
#[derive(Clone, Debug, Default)]
pub struct Asked {
    /// What asked for it: a console's user, Service Tools, PID 1.
    pub by: String,
    /// The product, its release and build.
    pub release: String,
    /// The machine's name and serial number.
    pub host: String,
    /// PID 1's status of the machine and each service, as text.
    pub status: String,
}

/// Whether a file named `name` holding `data` keeps a secret: by the
/// base's names, the product's `extra`, a `.key` file, or what it holds.
#[must_use]
pub fn secret(name: &str, data: &[u8], extra: &[(&str, &str)]) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    LEFT_OUT.iter().chain(extra).any(|(n, _)| *n == base)
        || Path::new(base)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("key"))
        || SECRET_MARKS
            .iter()
            .any(|m| data.windows(m.len()).any(|w| w == m.as_bytes()))
}

/// The archive's entries as they are gathered.
struct Gathered<W: Write> {
    zip: Writer<W>,
    extra: &'static [(&'static str, &'static str)],
    taken: Vec<(String, u64)>,
    left: Vec<(String, String)>,
}

impl<W: Write> Gathered<W> {
    fn add(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        if secret(name, data, self.extra) {
            let why = LEFT_OUT
                .iter()
                .chain(self.extra)
                .find(|(n, _)| name.ends_with(n))
                .map_or("it holds a secret", |(_, why)| why);
            self.left.push((name.to_string(), why.to_string()));
            return Ok(());
        }
        if self.zip.written() + data.len() as u64 > MAX_DUMP {
            self.left
                .push((name.to_string(), "the dump was at its limit".into()));
            return Ok(());
        }
        self.zip.add(name, data)?;
        self.taken.push((name.to_string(), data.len() as u64));
        Ok(())
    }

    /// A file's newest bytes, at most [`MAX_FILE`].
    fn file(&mut self, name: &str, path: &Path) -> std::io::Result<()> {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut f) = fs::File::open(path) else {
            return Ok(());
        };
        let len = f.metadata().map_or(0, |m| m.len());
        if len > MAX_FILE {
            f.seek(SeekFrom::Start(len - MAX_FILE))?;
        }
        let mut data = Vec::new();
        f.take(MAX_FILE).read_to_end(&mut data)?;
        self.add(name, &data)
    }

    /// Every file under `dir`, as `prefix/<its path>`, in name order.
    fn tree(&mut self, prefix: &str, dir: &Path, depth: usize) -> std::io::Result<()> {
        let Ok(read) = fs::read_dir(dir) else {
            return Ok(());
        };
        let mut all: Vec<_> = read.flatten().collect();
        all.sort_by_key(std::fs::DirEntry::file_name);
        for e in all {
            let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() && depth > 0 {
                self.tree(&name, &e.path(), depth - 1)?;
            } else if kind.is_file() {
                self.file(&name, &e.path())?;
            }
        }
        Ok(())
    }
}

/// Take a dump into `store`, at `now`: its name and size, and the names
/// of the older dumps put out to make room; or why it could not be
/// written (nothing of it is kept then).
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
    let kind = store.kind();
    let name = name_at(kind, now);
    fs::create_dir_all(store.dir()).map_err(|e| format!("{}: {e}", store.dir().display()))?;
    let gone = store.make_room(MAX_DUMP);
    let path = store.dir().join(&name);
    let tmp = store.dir().join(format!(".{name}.part"));
    let written = (|| -> std::io::Result<u64> {
        let file = fs::File::create(&tmp)?;
        let mut g = Gathered {
            zip: Writer::new(BufWriter::new(file), now),
            extra: kind.left_out,
            taken: Vec::new(),
            left: Vec::new(),
        };
        g.tree("log", &paths.log, 2)?;
        g.tree("problems", &paths.problems, 0)?;
        g.tree("state", &paths.state, 4)?;
        g.add("services/status.txt", asked.status.as_bytes())?;
        for (n, data) in sources.entries() {
            g.add(&n, &data)?;
        }
        for f in [
            "cpuinfo", "meminfo", "mounts", "cmdline", "uptime", "version",
        ] {
            g.file(&format!("host/{f}"), &paths.proc.join(f))?;
        }
        let mut manifest = format!(
            "{} {name}\nTaken {}\nBy {}\nRelease {}\nHost {}\n\nWhat it holds:\n",
            kind.title,
            appliance::time::rfc3339(now.saturating_mul(1_000_000_000)),
            asked.by,
            asked.release,
            asked.host,
        );
        for (n, bytes) in &g.taken {
            let _ = writeln!(manifest, "  {n}  {bytes} bytes");
        }
        manifest.push_str("\nLeft out:\n");
        if g.left.is_empty() {
            manifest.push_str("  nothing\n");
        }
        for (n, why) in &g.left {
            let _ = writeln!(manifest, "  {n}: {why}");
        }
        g.zip.add("MANIFEST.txt", manifest.as_bytes())?;
        let mut out = g.zip.finish(&format!("{BY}{}\n", asked.by))?;
        out.flush()?;
        let file = out
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?;
        file.sync_all()?;
        Ok(file.metadata()?.len())
    })();
    match written {
        Ok(bytes) => {
            fs::rename(&tmp, &path).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                format!("{name}: {e}")
            })?;
            Ok((
                Info {
                    name,
                    bytes,
                    time: u64::try_from(now).unwrap_or(0),
                    by: asked.by.clone(),
                },
                gone,
            ))
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(format!("{name}: {e}"))
        }
    }
}
