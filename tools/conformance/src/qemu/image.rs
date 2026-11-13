// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The files a guest is given: a copy of the disk image a boot may
//! write to, the A/B record in it, a seed volume, a blank disk.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use diskimg::efi::EspLocation;
use diskimg::layout::ESP_TYPE_GUID;
use diskimg::{SECTOR, fat};
use loadsource::layout::record_type;
use loadsource::record::{self, HostRecord};
use release::LOAD_SOURCE;
use uuid::Uuid;

/// The chunk a copy is made in; one that is all zero is left a hole.
const CHUNK: usize = 1 << 20;
/// The volume id of a seed volume the harness makes: `SEED`.
const SEED_VOLUME_ID: u32 = 0x5345_4544;

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

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

/// The partition of GPT type `ty` in `image`: its number, where it
/// starts and its length, in bytes.
fn partition(file: &mut File, ty: Uuid) -> Result<(u32, u64, u64), String> {
    let table = diskimg::read_table(file).map_err(text)?;
    let (index, _, first, last) = table
        .into_iter()
        .find(|p| p.1 == ty)
        .ok_or_else(|| format!("the image has no partition of type {ty}"))?;
    Ok((index, first * SECTOR, (last - first + 1) * SECTOR))
}

/// Where the ESP is on an image, by its table: a built image has the
/// fixed GUIDs, an installed disk its own.
pub fn esp(image: &Path) -> Result<EspLocation, String> {
    let mut file = File::open(image).map_err(text)?;
    let entry = loadsource::gpt::entries(&mut file)
        .map_err(text)?
        .into_iter()
        .find(|e| e.type_guid == ESP_TYPE_GUID)
        .ok_or("the image has no ESP")?;
    Ok(EspLocation {
        index: entry.index,
        first_lba: entry.first_lba,
        sectors: entry.last_lba + 1 - entry.first_lba,
        guid: entry.guid,
    })
}

/// Write `record` as a new A/B record of `image`: copy A, sequence 1.
pub fn set_record(image: &Path, record: HostRecord) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image)
        .map_err(text)?;
    let (_, start, _) = partition(&mut file, record_type(&LOAD_SOURCE))?;
    let bytes = record::initial_bytes(&LOAD_SOURCE, record).map_err(text)?;
    file.seek(SeekFrom::Start(start)).map_err(text)?;
    file.write_all(&bytes).map_err(text)?;
    file.sync_all().map_err(text)
}

/// The A/B record of `image` and its sequence.
pub fn record_of(image: &Path) -> Result<(HostRecord, u64), String> {
    let mut file = File::open(image).map_err(text)?;
    let (_, start, _) = partition(&mut file, record_type(&LOAD_SOURCE))?;
    let mut bytes = vec![0u8; 2 * record::spec(&LOAD_SOURCE).slot_bytes];
    file.seek(SeekFrom::Start(start)).map_err(text)?;
    file.read_exact(&mut bytes).map_err(text)?;
    record::read_bytes(&LOAD_SOURCE, &bytes).map_err(text)
}

/// Write a seed volume to `path`: a FAT file system labelled `label`
/// holding `seed.toml`.
pub fn seed_volume(path: &Path, label: &str, seed_toml: &str) -> Result<(), String> {
    let spec = fat::Spec {
        label: fat::label(label).map_err(text)?,
        volume_id: SEED_VOLUME_ID,
        ..fat::MEDIA_ESP
    };
    let volume = fat::build(&spec, &[("seed.toml", seed_toml.as_bytes())]).map_err(text)?;
    fs::write(path, volume).map_err(text)
}

/// Where the data partition of a host disk image is: its offset and
/// its length, in bytes.
pub fn data_partition(image: &Path) -> Result<(u64, u64), String> {
    let mut file = File::open(image).map_err(text)?;
    let (_, offset, bytes) = partition(&mut file, loadsource::layout::data_type(&LOAD_SOURCE))?;
    Ok((offset, bytes))
}

/// A disk of `bytes` zeros at `path`.
pub fn blank(path: &Path, bytes: u64) -> Result<(), String> {
    File::create(path)
        .and_then(|f| f.set_len(bytes))
        .map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sparse_copy_has_the_same_bytes() {
        let dir = std::env::temp_dir().join(format!("conformance-image-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("a.img"), dir.join("b.img"));
        // Data, a hole of several chunks, data that ends off a chunk.
        let mut bytes = vec![0u8; 5 * CHUNK + 123];
        bytes[..4].copy_from_slice(b"QSVS");
        bytes[5 * CHUNK + 100] = 7;
        fs::write(&from, &bytes).unwrap();
        sparse_copy(&from, &to).unwrap();
        assert_eq!(fs::read(&to).unwrap(), bytes);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_seed_volume_is_fat_with_the_label_and_the_file() {
        let dir = std::env::temp_dir().join(format!("conformance-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("seed.img");
        seed_volume(&path, release::SEED_LABEL, "schema_version = 1\n").unwrap();
        let volume = fs::read(&path).unwrap();
        assert_eq!(
            seed::volume::fat_label(&volume[..512]).as_deref(),
            Some(release::SEED_LABEL)
        );
        assert_eq!(
            fat::read_file(&volume, "seed.toml").unwrap(),
            b"schema_version = 1\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
