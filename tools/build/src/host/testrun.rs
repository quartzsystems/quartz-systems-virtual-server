// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build test`: run the conformance suite against the built images.

use anyhow::Result;
use buildkit::Ctx;
use buildkit::host::paths;
use tracing::{error, info, warn};

/// Run the suite for `media`. The serial log is printed on failure.
pub fn test(ctx: &Ctx, media: conformance::Media) -> Result<()> {
    let disk = paths::read_pointer(&ctx.paths.disk_pointer()).ok();
    let iso = paths::read_pointer(&ctx.paths.iso_pointer()).ok();
    let suite = conformance::Suite::new(
        disk.as_deref(),
        iso.as_deref(),
        media,
        ctx.paths.target.join("conformance"),
    )?;
    let l = suite.launch();
    info!(
        "test: {} with {} ({})",
        l.qemu.display(),
        l.firmware.display(),
        if l.kvm { "kvm, -cpu host" } else { "tcg" }
    );
    match suite.run_all() {
        Ok(reports) => {
            for r in &reports {
                info!(
                    "test: PASS {:<16} {:5.1}s  {}",
                    r.name,
                    r.elapsed.as_secs_f64(),
                    r.detail
                );
                if let Some(w) = &r.warning {
                    warn!("test: {}: {w}", r.name);
                }
            }
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
