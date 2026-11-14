// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/400 installed with nothing prepared outside the host: the media
//! repository designated, QS/400's ISO uploaded, a thin virtual disk
//! made, a partition with it and an optical device activated from IPL
//! source D; QS/400 installs itself unattended from the media, restarts
//! into side A, finishes from the media and runs; activated again with
//! `recorded`, it IPLs side A to completion.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use model::profile::{IplSource, MediaDevice};
use partd::Calls as _;
use poold::proto::Calls as _;

use super::{POOL_DISK_BYTES, QS400_ISO_ENV, disk_names, poold, said};
use crate::qemu::image::blank;
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::manager::{self, Spec};
use crate::suite::partition::qs400::{IPL_COMPLETE, ipl_complete};
use crate::suite::partition::{NAME, QS400_MEMORY_MIB, guest_line, power_off};

/// How long an installation may take, three levels deep.
const INSTALL_WITHIN: Duration = Duration::from_secs(3600);

/// QS/400's ISO the environment names: an ISO 9660 image with the
/// media UKI and an `INSTALL.RSP`, which makes the installation
/// unattended.
pub fn qs400_iso() -> Result<PathBuf, String> {
    let how = "build it with QS/400's `cargo run -p build -- iso` at the pinned commit, \
               with QS400_INSTALL_RSP naming a response file (docs/PHASE5.md)";
    let path = std::env::var_os(QS400_ISO_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{QS400_ISO_ENV} is not set: {how}"))?;
    let file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let mut v = ipl::iso::Volume::open(file, bytes)
        .map_err(|e| format!("{QS400_ISO_ENV}={}: {e}: {how}", path.display()))?;
    v.read_file(ipl::MEDIA_PATH, ipl::MAX_IMAGE_BYTES)
        .map_err(|e| format!("{}: no media UKI: {e}", path.display()))?;
    v.read_file("QS400/INSTALL.RSP", 64 * 1024)
        .map_err(|e| format!("{}: no INSTALL.RSP ({e}): {how}", path.display()))?;
    Ok(path)
}

/// `pool-install`.
pub fn install(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let iso = qs400_iso()?;
    let pool = run.file("pool0.img");
    blank(&pool, 4 * POOL_DISK_BYTES + (2 << 30))?;
    let mut guest = super::host(run, &[pool, iso.clone()])?;
    let names = disk_names(&mut guest, 2)?;
    said(poold(&mut guest)?.create_pool(
        "POOL1".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    super::pool_until(&mut guest, "POOL1", run.deadline(), |p| !p.mount.is_empty())?;
    said(poold(&mut guest)?.designate("POOL1".into()))?;
    let uploaded = channel(&mut guest)?.ask_within(
        &format!("upload QS400 {}", names[1]),
        Duration::from_mins(15),
    )?;
    said(poold(&mut guest)?.create_vdisk("POOL1".into(), "PRODLS".into(), 8192, false))?;

    let id = manager::partition(&mut guest, NAME)?;
    let mut spec = Spec::new(1, QS400_MEMORY_MIB, "POOL1/PRODLS");
    spec.name = "INSTALL".into();
    let mut profile = spec.profile();
    profile.optical = vec![MediaDevice {
        slot: 3,
        media: Some("QS400".into()),
    }];
    manager::said(manager::partd(&mut guest)?.put_profile(id, profile))?;
    let started = Instant::now();
    manager::said(manager::partd(&mut guest)?.activate(
        id,
        "INSTALL".into(),
        Some(IplSource::D),
        None,
    ))?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    guest.seen(&posted(&refcode::PARTITION_MEDIA_READ, ""), deadline)?;
    // The LIC phase restarts the partition, which is activated again
    // from the side the guest recorded: A, with the media still there.
    guest.seen(&posted(&refcode::PARTITION_RESTART, ""), deadline)?;
    guest.seen(
        &posted(&refcode::PARTITION_RECORD_READ, &format!("{NAME}, side A")),
        deadline,
    )?;
    ipl_complete(&mut guest, deadline)?;
    let installed = started.elapsed();
    let vdisk = poold(&mut guest)?
        .vdisks("POOL1".into())
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|d| d.name == "PRODLS")
        .ok_or("the virtual disk is gone")?;

    // Shut down, and activated again from the recorded side.
    manager::said(manager::partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    manager::said(manager::partd(&mut guest)?.activate(
        id,
        "INSTALL".into(),
        Some(IplSource::Recorded),
        None,
    ))?;
    let deadline = Instant::now() + Duration::from_mins(15);
    guest.seen(
        &posted(&refcode::PARTITION_STARTED, &format!("{NAME}, side A")),
        deadline,
    )?;
    ipl_complete(&mut guest, deadline)?;
    if guest.said(&guest_line("IPL source not set")) {
        return Err("the guest says its IPL source is not set".into());
    }
    manager::said(manager::partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "QS/400's ISO uploaded ({uploaded:?}), a thin virtual disk of 8 GiB made, the partition \
         activated from IPL source D: the media UKI read (`{}`), the LIC installed, the guest \
         restarted into side A and finished from the media to `{IPL_COMPLETE}` in {:.0} s, \
         {} bytes of the disk allocated; activated again with `recorded`, side A IPLed to \
         `{IPL_COMPLETE}`",
        refcode::PARTITION_MEDIA_READ.code,
        installed.as_secs_f64(),
        vdisk.allocated
    ))
}
