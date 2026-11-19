// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Disk images for a machine: blank ones, copies that leave what is
//! zero unwritten, and seed volumes.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use diskimg::fat;

/// The chunk a copy is made in; one that is all zero is left a hole.
const CHUNK: usize = 1 << 20;
/// The volume id of a seed volume made here: `SEED`.
const SEED_VOLUME_ID: u32 = 0x5345_4544;

/// Copy `from` to `to`, leaving what is zero unwritten: a 1 GiB image
/// that is mostly empty copies in a moment.
pub fn sparse_copy(from: &Path, to: &Path) -> Result<(), String> {
    let run = || -> std::io::Result<()> {
        let mut source = File::open(from)?;
        let mut copy = File::create(to)?;
        let len = source.metadata()?.len();
        copy.set_len(len)?;
        let mut chunk = vec![0u8; CHUNK];
        let mut at = 0u64;
        while at < len {
            let n = usize::try_from((len - at).min(CHUNK as u64)).unwrap_or(CHUNK);
            source.read_exact(&mut chunk[..n])?;
            if chunk[..n].iter().any(|b| *b != 0) {
                copy.seek(SeekFrom::Start(at))?;
                copy.write_all(&chunk[..n])?;
            }
            at += n as u64;
        }
        copy.sync_all()
    };
    run().map_err(|e| format!("copying {} to {}: {e}", from.display(), to.display()))
}

/// A disk of `bytes` zeros at `path`.
pub fn blank(path: &Path, bytes: u64) -> Result<(), String> {
    File::create(path)
        .and_then(|f| f.set_len(bytes))
        .map_err(|e| e.to_string())
}

/// Write a seed volume to `path`: a FAT file system labelled `label`
/// (the appliance's seed label) holding `seed.toml`.
pub fn seed_volume(path: &Path, label: &str, seed_toml: &str) -> Result<(), String> {
    let spec = fat::Spec {
        label: fat::label(label).map_err(|e| e.to_string())?,
        volume_id: SEED_VOLUME_ID,
        ..fat::MEDIA_ESP
    };
    let volume =
        fat::build(&spec, &[("seed.toml", seed_toml.as_bytes())]).map_err(|e| e.to_string())?;
    fs::write(path, volume).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qemu-disk-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_sparse_copy_has_the_same_bytes() {
        let dir = scratch("copy");
        let (from, to) = (dir.join("a.img"), dir.join("b.img"));
        // Data, a hole of several chunks, data that ends off a chunk.
        let mut bytes = vec![0u8; 5 * CHUNK + 123];
        bytes[..4].copy_from_slice(b"DATA");
        bytes[5 * CHUNK + 100] = 7;
        fs::write(&from, &bytes).unwrap();
        sparse_copy(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), bytes);
        blank(&from, 3 << 20).unwrap();
        assert_eq!(fs::read(&from).unwrap(), vec![0u8; 3 << 20]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_seed_volume_is_fat_with_the_label_and_the_file() {
        let dir = scratch("seed");
        let path = dir.join("seed.img");
        seed_volume(&path, "APPSEED", "schema_version = 1\n").unwrap();
        let volume = fs::read(&path).unwrap();
        assert_eq!(
            seed::volume::fat_label(&volume[..512]).as_deref(),
            Some("APPSEED")
        );
        assert_eq!(
            fat::read_file(&volume, "seed.toml").unwrap(),
            b"schema_version = 1\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
