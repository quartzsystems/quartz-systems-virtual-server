// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build test`: run the conformance suites against the built images.
//! Phase 0's suites boot the release image; the base system's boot the
//! test image, which has the hooks they need.

use anyhow::{Result, bail};
use buildkit::host::paths;
use buildkit::{Ctx, UkiKind};
use conformance::suite::Images;
use tracing::{error, info, warn};

/// Run the suites of `media`, or the one named `only`. The serial log
/// is printed on failure.
pub fn test(
    release: &Ctx,
    test: &Ctx,
    media: conformance::Media,
    only: Option<&str>,
) -> Result<()> {
    if only == Some("list") {
        for name in conformance::suite::names() {
            println!("{name}");
        }
        return Ok(());
    }
    let images = Images {
        release_disk: paths::read_pointer(&release.paths.disk_pointer()).ok(),
        iso: paths::read_pointer(&release.paths.iso_pointer()).ok(),
        test_disk: paths::read_pointer(&test.paths.disk_pointer()).ok(),
        test_iso: paths::read_pointer(&test.paths.iso_pointer()).ok(),
        test_ukis: paths::read_pointer(&test.paths.uki_pointer(UkiKind::SlotA))
            .ok()
            .zip(paths::read_pointer(&test.paths.uki_pointer(UkiKind::SlotB)).ok()),
    };
    if media == conformance::Media::Disk && images.test_disk.is_none() && only != Some("ipl") {
        bail!("the test image is not built; run `all --test-image` first");
    }
    let suite = conformance::Suite::new(images, release.paths.target.join("conformance"))?;
    let l = suite.launch();
    info!(
        "test: {} with {} ({})",
        l.qemu.display(),
        l.firmware.display(),
        if l.kvm { "kvm, -cpu host" } else { "tcg" }
    );
    let passed = |r: &conformance::Report| {
        info!(
            "test: PASS {:<14} {:6.1}s  {}",
            r.name,
            r.elapsed.as_secs_f64(),
            r.detail
        );
        if let Some(w) = &r.warning {
            warn!("test: {}: {w}", r.name);
        }
    };
    match suite.run(media, only, passed) {
        Ok(reports) => {
            info!("test: PASS, {} step(s), media {media}", reports.len());
            Ok(())
        }
        Err(e) => {
            if let Some(log) = e.log() {
                eprintln!("----- serial log -----\n{log}\n----- end serial log -----");
            }
            error!("test: FAIL: {e}");
            Err(e.into())
        }
    }
}
