// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! State across an update (`docs/UPDATE.md`), before any service or the
//! seed reads it. The pending slot's first IPL keeps a copy of
//! `/data/state`; a slot that confirms drops it; a slot that comes back
//! after the pending one did not confirm puts it back. A power loss at
//! any point leaves either the state or the copy whole, and the next
//! IPL finishes what was begun.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The copy, under the data file system.
pub const COPY: &str = "update/state-copy";

/// What was done to the state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    /// Nothing: no update is under way.
    Nothing,
    /// The pending slot's first IPL: the state was copied.
    Copied,
    /// The pending slot again: the copy made at its first IPL is kept.
    Kept,
    /// The copy was put back: the pending slot did not confirm.
    Restored,
    /// A copy left from an update that ended was dropped.
    Dropped,
}

/// What this IPL is, as the state step needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipl {
    /// Nothing is pending, or a slot is installed and not activated.
    Normal,
    /// The pending slot's trial.
    Trial,
    /// The slot that came back after the pending one did not confirm.
    FellBack,
}

fn paths(data: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let copy = data.join(COPY);
    (
        data.join("state"),
        copy.clone(),
        copy.with_extension("part"),
    )
}

/// Copy the tree `from` to `to`, each file synchronized.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &target)?;
        } else {
            fs::copy(e.path(), &target)?;
            fs::File::open(&target)?.sync_all()?;
        }
    }
    fs::File::open(to)?.sync_all()
}

fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// The state step's first act, for an IPL that is `ipl`, the data file
/// system at `data`: what was done.
///
/// # Errors
/// A copy or a rename failed; the state is as it was.
pub fn before_state(data: &Path, ipl: Ipl) -> io::Result<Done> {
    let (state, copy, part) = paths(data);
    let parent = copy.parent().unwrap_or(data).to_path_buf();
    // A copy begun and not finished is no copy.
    if part.exists() {
        fs::remove_dir_all(&part)?;
    }
    // A restore cut short between its renames: the copy is the state.
    if !state.exists() && copy.exists() {
        fs::rename(&copy, &state)?;
        sync_dir(data);
        return Ok(Done::Restored);
    }
    match ipl {
        Ipl::Trial if copy.exists() => Ok(Done::Kept),
        Ipl::Trial => {
            fs::create_dir_all(&parent)?;
            if state.exists() {
                copy_tree(&state, &part)?;
            } else {
                fs::create_dir_all(&part)?;
            }
            fs::rename(&part, &copy)?;
            sync_dir(&parent);
            Ok(Done::Copied)
        }
        Ipl::FellBack if copy.exists() => {
            let trial = data.join("state.trial");
            if trial.exists() {
                fs::remove_dir_all(&trial)?;
            }
            if state.exists() {
                fs::rename(&state, &trial)?;
            }
            fs::rename(&copy, &state)?;
            sync_dir(data);
            let _ = fs::remove_dir_all(&trial);
            Ok(Done::Restored)
        }
        Ipl::Normal if copy.exists() => {
            fs::remove_dir_all(&copy)?;
            Ok(Done::Dropped)
        }
        Ipl::FellBack | Ipl::Normal => Ok(Done::Nothing),
    }
}

/// The pending slot confirmed: its copy is dropped.
///
/// # Errors
/// It could not be removed; the next IPL drops it.
pub fn confirmed(data: &Path) -> io::Result<()> {
    let (_, copy, _) = paths(data);
    match fs::remove_dir_all(&copy) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("state-copy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("state/partitions/1")).unwrap();
        fs::write(d.join("state/host.toml"), "before").unwrap();
        fs::write(d.join("state/partitions/1/definition.toml"), "PROD").unwrap();
        d
    }

    fn read(d: &Path, f: &str) -> String {
        fs::read_to_string(d.join("state").join(f)).unwrap_or_default()
    }

    #[test]
    fn a_rolled_back_host_is_exactly_as_it_was() {
        let d = scratch("back");
        assert_eq!(before_state(&d, Ipl::Trial).unwrap(), Done::Copied);
        // The trial's release changes and adds state.
        fs::write(d.join("state/host.toml"), "after").unwrap();
        fs::write(d.join("state/new.toml"), "new").unwrap();
        // A second IPL of the trial keeps the first copy.
        assert_eq!(before_state(&d, Ipl::Trial).unwrap(), Done::Kept);
        assert_eq!(before_state(&d, Ipl::FellBack).unwrap(), Done::Restored);
        assert_eq!(read(&d, "host.toml"), "before");
        assert_eq!(read(&d, "partitions/1/definition.toml"), "PROD");
        assert!(!d.join("state/new.toml").exists());
        assert!(!d.join(COPY).exists() && !d.join("state.trial").exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_confirmed_slot_drops_the_copy() {
        let d = scratch("confirm");
        before_state(&d, Ipl::Trial).unwrap();
        fs::write(d.join("state/host.toml"), "after").unwrap();
        confirmed(&d).unwrap();
        assert!(!d.join(COPY).exists());
        assert_eq!(read(&d, "host.toml"), "after");
        assert_eq!(before_state(&d, Ipl::Normal).unwrap(), Done::Nothing);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_power_loss_anywhere_leaves_the_state_whole() {
        // Cut inside the copy: the part is no copy.
        let d = scratch("cut");
        fs::create_dir_all(d.join(COPY).with_extension("part")).unwrap();
        assert_eq!(before_state(&d, Ipl::FellBack).unwrap(), Done::Nothing);
        assert_eq!(read(&d, "host.toml"), "before");
        // Cut between a restore's renames: the state is gone, the copy whole.
        before_state(&d, Ipl::Trial).unwrap();
        fs::rename(d.join("state"), d.join("state.trial")).unwrap();
        assert_eq!(before_state(&d, Ipl::Normal).unwrap(), Done::Restored);
        assert_eq!(read(&d, "host.toml"), "before");
        // A copy left after an update that ended is dropped.
        before_state(&d, Ipl::Trial).unwrap();
        assert_eq!(before_state(&d, Ipl::Normal).unwrap(), Done::Dropped);
        fs::remove_dir_all(&d).unwrap();
    }
}
