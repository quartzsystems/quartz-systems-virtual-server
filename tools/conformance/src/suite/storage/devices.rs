// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The optical and tape devices over media of the library. A guest's
//! own reads and writes of them go through a driver the suite does not
//! control, so the commands are made here, through the test hooks'
//! probe: the device's own request handling over a virtqueue, with the
//! library's files behind it as a partition has them.

use std::time::Duration;

use poold::proto::Calls as _;

use super::{POOL_DISK_BYTES, disk_names, poold, said};
use crate::qemu::image::blank;
use crate::suite::base::{Run, channel};
use crate::suite::partition::power_off;

/// The host with a pool that is the media repository, and further
/// drives `images`, each uploaded as an image: their names.
fn library<'a>(
    run: &Run<'a>,
    images: &[(&str, Vec<u8>)],
) -> Result<crate::suite::base::Guest<'a>, String> {
    let pool = run.file("pool0.img");
    blank(&pool, POOL_DISK_BYTES)?;
    let mut drives = vec![pool];
    for (name, bytes) in images {
        let path = run.file(&format!("{name}.iso"));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        drives.push(path);
    }
    let mut guest = super::host(run, &drives)?;
    let names = disk_names(&mut guest, drives.len())?;
    said(poold(&mut guest)?.create_pool(
        "MEDIA".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    super::pool_until(&mut guest, "MEDIA", run.deadline(), |p| !p.mount.is_empty())?;
    said(poold(&mut guest)?.designate("MEDIA".into()))?;
    for ((name, _), device) in images.iter().zip(&names[1..]) {
        channel(&mut guest)?.ask(&format!("upload {name} {device}"))?;
    }
    Ok(guest)
}

/// `pool-optical`.
pub fn optical(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let filler: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let first = ipl::iso::build("FIRST", &[("DATA.BIN", filler.as_slice())]);
    let second = ipl::iso::build("SECOND", &[("README.TXT", b"another".as_slice())]);
    let mut guest = library(run, &[("FIRST", first), ("SECOND", second)])?;
    let dir = "/pools/MEDIA/media/optical";
    let lines = channel(&mut guest)?.ask(&format!(
        "scsiprobe optical {dir}/FIRST.iso {dir}/SECOND.iso"
    ))?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(lines.join("; "))
}

/// `pool-tape`: the device's own behaviour. A save from QS/400 and its
/// restore to another partition are `docs/PHASE5.md`'s.
pub fn tape(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let mut guest = library(run, &[])?;
    said(poold(&mut guest)?.create_volume("SAVE1".into(), "SAV001".into(), 16))?;
    let lines = channel(&mut guest)?.ask("scsiprobe tape /pools/MEDIA/media/tape/SAVE1.aws")?;
    let listed = poold(&mut guest)?.volumes().map_err(|e| e.to_string())?;
    let used = listed
        .iter()
        .find(|v| v.name == "SAVE1")
        .map_or(0, |v| v.used);
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "{}; the volume holds {used} bytes",
        lines.join("; ")
    ))
}
