// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the partition manager keeps in files beside its state: each
//! partition's history of reference codes, and its dumps in the pool
//! that holds the media repository.
//!
//! ```text
//! <state>/partitions/<ID>/history            the history, an ipc value, written whole
//! <repository>/dumps/<ID>/<NAME>-<time>.core  a dump (docs/DUMPS.md)
//! ```

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;

use crate::proto::{DumpInfo, Posted};

/// The history's file in a partition's state directory.
pub const HISTORY: &str = "history";
/// The dumps' directory in the repository's pool.
pub const DUMPS: &str = "dumps";
/// The most bytes of a history read back.
const MAX_HISTORY: u64 = 1 << 20;

/// The history file of partition `id` under `state`.
pub fn history_path(state: &Path, id: u32) -> PathBuf {
    state
        .join(crate::store::DIR)
        .join(id.to_string())
        .join(HISTORY)
}

/// The history kept at `path`; none when it cannot be read.
pub fn read_history(path: &Path) -> Vec<Posted> {
    let Ok(meta) = fs::metadata(path) else {
        return Vec::new();
    };
    if meta.len() > MAX_HISTORY {
        return Vec::new();
    }
    fs::read(path)
        .ok()
        .and_then(|bytes| ipc::decode::<Vec<Posted>>(&bytes).ok())
        .unwrap_or_default()
}

/// A thread that writes the histories it is sent, the latest of each
/// partition: the manager never waits on the disk.
pub fn history_writer(state: PathBuf) -> Sender<(u32, Vec<Posted>)> {
    let (to, from) = mpsc::channel::<(u32, Vec<Posted>)>();
    thread::spawn(move || {
        while let Ok(first) = from.recv() {
            let mut latest: BTreeMap<u32, Vec<Posted>> = BTreeMap::new();
            latest.insert(first.0, first.1);
            while let Ok((id, history)) = from.try_recv() {
                latest.insert(id, history);
            }
            for (id, history) in latest {
                let path = history_path(&state, id);
                // A partition deleted meanwhile has no directory.
                if path.parent().is_some_and(Path::exists)
                    && let Err(e) = state::store::atomic_write(&path, &ipc::encode(&history))
                {
                    println!("{}: {e}", path.display());
                }
            }
        }
    });
    to
}

/// The dumps in `dir`, oldest first: the files whose header reads.
pub fn dumps_in(dir: &Path) -> Vec<DumpInfo> {
    let mut out: Vec<DumpInfo> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !crate::manager::valid_dump_name(&name) {
                return None;
            }
            let mut file = File::open(entry.path()).ok()?;
            let meta = file.metadata().ok()?;
            let header = dump::read_header(&mut file).ok()?;
            Some(DumpInfo {
                name,
                time: header.time,
                bytes: meta.len(),
                allocated: meta.blocks() * 512,
                code: header.code,
                reason: header.reason,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.time, &a.name).cmp(&(b.time, &b.name)));
    out
}

/// Remove what is left of dumps that were not finished: a partition
/// manager that ended while one was written.
pub fn clear_partial(dir: &Path) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().ends_with(dump::PARTIAL) {
            let _ = fs::remove_file(entry.path());
        }
    }
}
