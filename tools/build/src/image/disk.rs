// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build disk`: `buildkit` writes the GPT, the ESP with both slots and
//! the empty `QSVSDATA`; this adds the first A/B record and reads the
//! image back: the record, and an ESP that is QS/VS's own and holds
//! exactly the two UKIs.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use buildkit::Ctx;
use buildkit::host::hash::sha256_file;
use diskimg::layout::ESP_TYPE_GUID;
use diskimg::{SECTOR, fat};
use loadsource::layout;
use loadsource::record::{self, HostRecord};
use release::LOAD_SOURCE;
use tracing::info;
use uuid::Uuid;

/// Offsets of the volume id and label in a FAT32 boot sector.
const FAT32_VOLUME_ID: usize = 67;
const FAT32_LABEL: usize = 71;

/// Where a partition is in the image: byte offset and length.
type Extent = (u64, u64);

fn extent(table: &[(u32, Uuid, u64, u64)], ty: Uuid) -> Result<Extent> {
    let (_, _, first, last) = table
        .iter()
        .find(|p| p.1 == ty)
        .with_context(|| format!("the image has no partition of type {ty}"))?;
    Ok((first * SECTOR, (last - first + 1) * SECTOR))
}

fn read_at(file: &mut File, at: u64, len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; len];
    file.seek(SeekFrom::Start(at))?;
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Write the first record: slot A the default, sequence 1 in copy A.
fn write_record(file: &mut File, (at, len): Extent) -> Result<()> {
    let bytes = record::initial_bytes(&LOAD_SOURCE, HostRecord::INITIAL)?;
    ensure!(
        bytes.len() as u64 <= len,
        "the record partition is {len} bytes, the record needs {}",
        bytes.len()
    );
    file.seek(SeekFrom::Start(at))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let back = read_at(file, at, bytes.len())?;
    let (found, sequence) = record::read_bytes(&LOAD_SOURCE, &back)?;
    ensure!(
        found == HostRecord::INITIAL && sequence == 1,
        "the record reads back as {found:?}, sequence {sequence}"
    );
    Ok(())
}

/// The ESP as it is in the image: QS/VS's label and volume id, and
/// exactly the two UKIs.
fn check_esp(file: &mut File, (at, len): Extent) -> Result<()> {
    let esp = read_at(file, at, usize::try_from(len)?)?;
    let id = u32::from_le_bytes(esp[FAT32_VOLUME_ID..FAT32_VOLUME_ID + 4].try_into()?);
    let label = &esp[FAT32_LABEL..FAT32_LABEL + 11];
    ensure!(
        label == fat::label(LOAD_SOURCE.esp_label)? && id == LOAD_SOURCE.esp_volume_id,
        "the ESP is `{}` with volume id {id:#010x}",
        String::from_utf8_lossy(label)
    );
    let listing = fat::list(&esp)?;
    let expected = [
        "EFI/".to_string(),
        "EFI/BOOT/".to_string(),
        layout::SLOT_A_PATH.to_string(),
        format!("EFI/{}/", LOAD_SOURCE.vendor),
        LOAD_SOURCE.slot_b_path.to_string(),
    ];
    ensure!(listing == expected, "ESP contents unexpected: {listing:?}");
    Ok(())
}

/// `build disk`: an image of `bytes` (1 GiB when `None`).
pub fn disk(ctx: &Ctx, bytes: Option<u64>) -> Result<PathBuf> {
    let out = buildkit::image::disk::disk(ctx, bytes)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&out)
        .with_context(|| format!("cannot open {}", out.display()))?;
    let table = diskimg::read_table(&mut file)?;
    write_record(
        &mut file,
        extent(&table, layout::record_type(&LOAD_SOURCE))?,
    )?;
    check_esp(&mut file, extent(&table, ESP_TYPE_GUID)?)?;
    drop(file);
    info!(
        "disk: A/B record `{}` written (slot A, sequence 1); ESP `{}` holds the two UKIs; sha256 {}",
        String::from_utf8_lossy(&LOAD_SOURCE.record_magic).trim_end_matches('\0'),
        LOAD_SOURCE.esp_label,
        sha256_file(&out)?
    );
    Ok(out)
}
