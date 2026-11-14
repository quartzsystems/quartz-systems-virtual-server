// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The media library: uploads whole, cut off and wrong; and media that
//! IPL source D cannot start from.

use std::path::PathBuf;
use std::time::Duration;

use model::profile::{IplSource, MediaDevice};
use partd::Calls as _;
use poold::proto::Calls as _;

use super::{POOL_DISK_BYTES, disk_names, poold, said};
use crate::qemu::image::blank;
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::manager::{self, Spec};
use crate::suite::partition::{NAME, QSVS_MEMORY_MIB, power_off};

/// An image file for an extra drive: `blocks` of 2048 bytes made by
/// `make`.
fn image_file(run: &Run<'_>, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = run.file(name);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The host with a pool of one disk that is the media repository, and
/// further drives `media`, each an image to upload; their names.
fn library_host<'a>(
    run: &Run<'a>,
    media: &[PathBuf],
) -> Result<(crate::suite::base::Guest<'a>, Vec<String>), String> {
    let pool = run.file("pool0.img");
    blank(&pool, POOL_DISK_BYTES)?;
    let mut drives = vec![pool];
    drives.extend_from_slice(media);
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
    Ok((guest, names[1..].to_vec()))
}

fn send_image(guest: &mut Session, name: &str, device: &str) -> Result<Vec<String>, String> {
    channel(guest)?.ask(&format!("upload {name} {device}"))
}

/// `pool-upload`: an interrupted upload leaves nothing; a file that is
/// not an ISO 9660 image is refused; a whole one is kept.
pub fn upload_suite(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let good = ipl::iso::build("GOOD_IMAGE", &[("README.TXT", b"a medium".as_slice())]);
    let mut bad = vec![0u8; good.len()];
    bad[..4].copy_from_slice(b"junk");
    let media = vec![
        image_file(run, "good.iso", &good)?,
        image_file(run, "bad.iso", &bad)?,
    ];
    let (mut guest, devices) = library_host(run, &media)?;
    // Cut off after a megabyte's worth: nothing is kept.
    channel(&mut guest)?.ask(&format!("upload CUT {} 20480", devices[0]))?;
    std::thread::sleep(Duration::from_secs(1));
    let left = channel(&mut guest)?
        .ls("/pools/MEDIA/media/optical")
        .unwrap_or_default();
    if !left.is_empty() {
        return Err(format!("a cut-off upload left {left:?}"));
    }
    let refused = send_image(&mut guest, "BAD", &devices[1])
        .err()
        .unwrap_or_default();
    if !refused.contains("QVS3206") {
        return Err(format!(
            "a file that is not an ISO was answered `{refused}`"
        ));
    }
    let kept = send_image(&mut guest, "GOOD", &devices[0])?;
    let images = poold(&mut guest)?.images().map_err(|e| e.to_string())?;
    let listed: Vec<_> = images
        .iter()
        .map(|i| (i.name.clone(), i.label.clone()))
        .collect();
    if listed != [("GOOD".to_string(), "GOOD_IMAGE".to_string())] {
        return Err(format!("the library has {listed:?}"));
    }
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "an upload cut off after 20 KiB left nothing; a file that is not an ISO 9660 image was \
         refused with QVS3206 and left nothing; a whole one was kept ({kept:?}) and listed by its \
         volume identifier"
    ))
}

/// `pool-media-damaged`: IPL source D with no medium, with a medium
/// that is not an ISO, and with one that has no UKI: each its code, and
/// the VMM ends in order.
pub fn damaged(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let no_uki = ipl::iso::build("NO_UKI", &[("README.TXT", b"no kernel".as_slice())]);
    let mut not_iso = no_uki.clone();
    not_iso[16 * 2048 + 1] = b'X';
    let media = vec![
        image_file(run, "no-uki.iso", &no_uki)?,
        image_file(run, "not-iso.iso", &not_iso)?,
    ];
    let (mut guest, devices) = library_host(run, &media)?;
    send_image(&mut guest, "NOUKI", &devices[0])?;
    // An image that is not an ISO is refused by the library: one that
    // passed and was damaged afterwards is written in place.
    send_image(&mut guest, "DAMAGED", &devices[0])?;
    channel(&mut guest)?.ask(&format!(
        "copy /dev/{} 0 /pools/MEDIA/media/optical/DAMAGED.iso 0 {}",
        devices[1],
        not_iso.len()
    ))?;
    said(poold(&mut guest)?.create_vdisk("MEDIA".into(), "DISK".into(), 64, false))?;
    let id = manager::partition(&mut guest, NAME)?;
    let mut seen = Vec::new();
    for (medium, code) in [
        (None, &refcode::PARTITION_NO_MEDIA),
        (Some("DAMAGED"), &refcode::PARTITION_BAD_MEDIA),
        (Some("NOUKI"), &refcode::PARTITION_NO_MEDIA_IMAGE),
    ] {
        let mut spec = Spec::new(1, QSVS_MEMORY_MIB, "MEDIA/DISK").side(IplSource::D);
        spec.name = "INSTALL".into();
        let mut profile = spec.profile();
        profile.optical = vec![MediaDevice {
            slot: 3,
            media: medium.map(str::to_string),
        }];
        manager::said(manager::partd(&mut guest)?.put_profile(id, profile))?;
        manager::said(manager::partd(&mut guest)?.activate(id, "INSTALL".into(), None, None))?;
        guest.seen(&posted(code, ""), run.deadline())?;
        let ended = manager::ended(&mut guest, id, run.deadline())?;
        seen.push(format!(
            "{}: {} then {}",
            medium.unwrap_or("none"),
            code.code,
            ended.state.word()
        ));
    }
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "IPL source D from an empty drive, a damaged image and one with no UKI: {}",
        seen.join("; ")
    ))
}
