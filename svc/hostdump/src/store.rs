// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's dumps on `/data`: one archive each, named by when it was
//! taken, bounded in number and in bytes, the oldest put out first.
//! What asked for one is its archive's comment.

use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// The directory under the data file system.
pub const DIR: &str = "dumps/host";
/// The most dumps kept.
pub const MAX_DUMPS: usize = 8;
/// The most bytes they may take together.
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
/// A dump's name: its prefix, then `YYYYMMDD-HHMMSS`, then [`EXT`].
pub const PREFIX: &str = "HOSTDUMP-";
/// Its extension.
pub const EXT: &str = ".zip";
/// The comment's prefix that says who asked.
pub const BY: &str = "by: ";

/// One dump, as a list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// When it was taken, seconds since the epoch.
    pub time: u64,
    /// What asked for it.
    pub by: String,
}
ipc::wire_struct!(Info {
    name,
    bytes,
    time,
    by
});

/// Whether `name` is one of a dump's: nothing else is ever opened or
/// removed by name.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    let Some(stamp) = name.strip_prefix(PREFIX).and_then(|n| n.strip_suffix(EXT)) else {
        return false;
    };
    let b = stamp.as_bytes();
    b.len() == 15
        && b[8] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit())
}

/// The name of a dump taken at `unix_secs`.
#[must_use]
pub fn name_at(unix_secs: i64) -> String {
    let c = appliance::time::civil(unix_secs);
    format!(
        "{PREFIX}{:04}{:02}{:02}-{:02}{:02}{:02}{EXT}",
        c.year, c.month, c.day, c.hour, c.minute, c.second
    )
}

/// The time a dump's name says, seconds since the epoch.
fn time_of(name: &str) -> u64 {
    let digits: Vec<i64> = name
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| i64::from(b - b'0'))
        .collect();
    if digits.len() != 14 {
        return 0;
    }
    let n = |r: std::ops::Range<usize>| digits[r].iter().fold(0, |a, d| a * 10 + d);
    let secs =
        appliance::time::from_civil(n(0..4), n(4..6), n(6..8), n(8..10), n(10..12), n(12..14));
    u64::try_from(secs).unwrap_or(0)
}

/// The dumps kept in a directory.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store in `dir`.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Its directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The path of the dump `name`; none when the name is no dump's.
    #[must_use]
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        valid_name(name).then(|| self.dir.join(name))
    }

    /// Every dump, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<Info> {
        let Ok(dir) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut all: Vec<Info> = dir
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                if !valid_name(&name) {
                    return None;
                }
                let bytes = e.metadata().ok()?.len();
                let by = File::open(e.path())
                    .ok()
                    .and_then(|mut f| crate::zip::comment(&mut f))
                    .and_then(|c| {
                        c.lines()
                            .find_map(|l| l.strip_prefix(BY).map(str::to_string))
                    })
                    .unwrap_or_default();
                Some(Info {
                    time: time_of(&name),
                    name,
                    bytes,
                    by,
                })
            })
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        all
    }

    /// Delete the dump `name`.
    ///
    /// # Errors
    /// No such dump, or it could not be removed.
    pub fn delete(&self, name: &str) -> Result<(), String> {
        let path = self
            .path(name)
            .ok_or_else(|| format!("{name}: no such dump"))?;
        fs::remove_file(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{name}: no such dump"),
            _ => format!("{name}: {e}"),
        })
    }

    /// Put out the oldest until one of `bytes` more fits the bounds:
    /// the names put out.
    pub fn make_room(&self, bytes: u64) -> Vec<String> {
        let mut all = self.list();
        let mut gone = Vec::new();
        let total = |all: &[Info]| all.iter().map(|i| i.bytes).sum::<u64>();
        while !all.is_empty()
            && (all.len() + 1 > MAX_DUMPS || total(&all).saturating_add(bytes) > MAX_TOTAL)
        {
            let old = all.remove(0);
            if self.delete(&old.name).is_ok() {
                gone.push(old.name);
            }
        }
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_says_when_and_nothing_else_is_one() {
        let n = name_at(1_794_834_942);
        assert_eq!(n, "HOSTDUMP-20261116-131542.zip");
        assert!(valid_name(&n));
        assert_eq!(time_of(&n), 1_794_834_942);
        for bad in [
            "../x",
            "HOSTDUMP-2026111-131542.zip",
            "HOSTDUMP-20261116-131542.zip/..",
            "identity.key",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn the_oldest_go_first_at_the_bounds() {
        let d = std::env::temp_dir().join(format!("hostdump-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let s = Store::at(&d);
        for i in 0..i64::try_from(MAX_DUMPS).unwrap() {
            let mut w = crate::zip::Writer::new(Vec::new(), 0);
            w.add("x", b"y").unwrap();
            let bytes = w.finish("by: test\n").unwrap();
            fs::write(d.join(name_at(1_794_834_942 + i)), bytes).unwrap();
        }
        fs::write(d.join("identity.key"), "not a dump").unwrap();
        let all = s.list();
        assert_eq!((all.len(), all[0].by.as_str()), (MAX_DUMPS, "test"));
        let gone = s.make_room(10);
        assert_eq!(gone, [all[0].name.clone()]);
        assert!(s.delete("identity.key").is_err(), "only a dump's name");
        assert!(d.join("identity.key").exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
