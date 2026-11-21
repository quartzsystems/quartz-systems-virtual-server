// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build update`: QS/VS's update image (`docs/UPDATE.md`), made by
//! `base/buildaid` in QS/VS's format, naming the state files this
//! release reads. The artifact manifest lists it.

use std::path::PathBuf;

use anyhow::Result;
use buildkit::Ctx;

pub use buildaid::updimg::{POINTER, list};

/// The fixes `QSVS_FIXES` names, checked.
pub fn named_fixes(ctx: &Ctx) -> Result<Vec<String>> {
    buildaid::updimg::named_fixes(ctx, &qsvsupdd::FORMAT)
}

/// `build update`: the image's path.
pub fn update(ctx: &Ctx) -> Result<PathBuf> {
    buildaid::updimg::update(ctx, &qsvsupdd::FORMAT, qsvsupdd::schemas::this_release())
}
