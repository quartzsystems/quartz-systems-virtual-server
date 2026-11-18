// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's release (this repository's `VERSION`, or `QSVS_RELEASE`) and
//! build ID (`git describe`, or `QSVS_BUILD_ID`), under `QSVS_`; and
//! which kind of key signs the image (`QSVS_KEY_KIND`, which the build's
//! userland step sets: `release`, or `development` when no signing key
//! was given).

use std::path::Path;

fn main() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the release crate is under lic/");
    vrm::build::capture(repo, "QSVS_");
    println!("cargo::rerun-if-env-changed=QSVS_KEY_KIND");
    let kind = std::env::var("QSVS_KEY_KIND").unwrap_or_default();
    // Built outside the pipeline (a unit test, a developer's cargo):
    // no release key signed it.
    let kind = if kind == "release" {
        "release"
    } else {
        "development"
    };
    println!("cargo::rustc-env=QSVS_KEY_KIND={kind}");
}
