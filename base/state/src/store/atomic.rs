// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The atomic write: a temporary file in the same directory, synced,
//! renamed over the target, and the directory synced. After a power
//! loss the target is the whole old file or the whole new one.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// Replace `path` with `bytes`.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the file has no directory"))?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("the file has no name"))?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp"));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(dir)
}

/// Make the rename durable. A directory opens for reading on Unix only.
fn sync_dir(dir: &Path) -> io::Result<()> {
    if cfg!(unix) {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_replaced_and_no_temporary_is_left() {
        let dir = std::env::temp_dir().join(format!("state-atomic-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.toml");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        // A stale temporary from an interrupted write is written over.
        fs::write(dir.join(".a.toml.tmp"), b"partial").unwrap();
        atomic_write(&path, b"three").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"three");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
