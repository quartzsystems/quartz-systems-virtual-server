// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's release (this repository's `VERSION`, or `QSVS_RELEASE`) and
//! build ID (`git describe`, or `QSVS_BUILD_ID`), under `QSVS_`.

use std::path::Path;

fn main() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the release crate is under lic/");
    vrm::build::capture(repo, "QSVS_");
}
