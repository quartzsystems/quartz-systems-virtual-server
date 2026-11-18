// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A host that restarted after a kernel panic (`docs/SECURITY.md`). An
//! oops is a panic and a panic restarts the host (`panic=-1`); as it
//! panics, the kernel writes its last messages into the firmware's
//! variables (`efi-pstore`). At the next IPL they are moved from
//! `/sys/fs/pstore` into `/data/log/panics`, which the next host dump
//! takes, the firmware's variables are freed, and the panic is posted
//! (`A2001005`), which opens its problem. The most recent ones are kept.

use std::fs;
use std::path::Path;

/// Where the kernel shows what it kept.
pub const PSTORE: &str = "/sys/fs/pstore";
/// How many panics' files are kept.
const KEPT: usize = 8;

/// The line that says why the kernel panicked, when the records have it.
pub fn why(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_once("Kernel panic - not syncing: "))
        .map(|(_, why)| why.trim().to_string())
        .next()
}

/// The names of the files of the panics kept in `dir`, oldest first,
/// grouped by the panic's time they start with.
fn kept(dir: &Path) -> Vec<String> {
    let mut times: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.split_once('-').map(|(t, _)| t.to_string())
        })
        .collect();
    times.sort();
    times.dedup();
    times
}

/// Move what the kernel kept from `pstore` into `dir`, named after
/// `time`; what it says. `None` when nothing was kept.
pub fn take(pstore: &Path, dir: &Path, time: &str) -> Option<(String, usize)> {
    let mut entries: Vec<_> = fs::read_dir(pstore)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    if entries.is_empty() {
        return None;
    }
    entries.sort();
    let _ = fs::create_dir_all(dir);
    let mut why_it = None;
    let mut moved = 0;
    for path in &entries {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let Ok(bytes) = fs::read(path) else { continue };
        if why_it.is_none() {
            why_it = why(&String::from_utf8_lossy(&bytes));
        }
        if fs::write(dir.join(format!("{time}-{name}")), &bytes).is_ok() {
            moved += 1;
            // Frees the firmware's variable for the next one.
            let _ = fs::remove_file(path);
        }
    }
    // The oldest panics' files go once more than KEPT are there.
    let times = kept(dir);
    if times.len() > KEPT {
        let old = &times[..times.len() - KEPT];
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if old.iter().any(|t| name.starts_with(&format!("{t}-"))) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    Some((
        why_it.unwrap_or_else(|| "the kernel's last messages were kept".into()),
        moved,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("panics-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn what_the_kernel_kept_is_moved_and_its_reason_found() {
        let pstore = scratch("store");
        let dir = scratch("kept");
        assert_eq!(take(&pstore, &dir, "20261118T150000"), None);
        fs::write(
            pstore.join("dmesg-efi_pstore-1"),
            "<0>[ 9.1] Kernel panic - not syncing: Fatal exception\n<4>[ 9.2] RIP: x\n",
        )
        .unwrap();
        fs::write(pstore.join("dmesg-efi_pstore-2"), "<6>[ 8.0] earlier\n").unwrap();
        let (said, moved) = take(&pstore, &dir, "20261118T150000").unwrap();
        assert_eq!(said, "Fatal exception");
        assert_eq!(moved, 2);
        assert_eq!(fs::read_dir(&pstore).unwrap().count(), 0);
        assert!(dir.join("20261118T150000-dmesg-efi_pstore-1").is_file());
        // Only the most recent panics are kept.
        for n in 0..KEPT + 2 {
            fs::write(pstore.join("dmesg-efi_pstore-9"), "x").unwrap();
            take(&pstore, &dir, &format!("20261119T{n:06}")).unwrap();
        }
        assert_eq!(kept(&dir).len(), KEPT);
        fs::remove_dir_all(&pstore).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
