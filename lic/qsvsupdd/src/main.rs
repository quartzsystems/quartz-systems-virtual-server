// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's update service (`docs/UPDATE.md`): `svc/update` run as
//! QS/VS, with the release that runs, the keys its image trusts, its
//! load-source names, and its directory on `/data`.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::{Path, PathBuf};

    let host = update::serve::Host {
        post: refcode::post::remote::code_about,
        share: release::share_with_server,
        run_dir: release::RUN_DIR,
        dir: Path::new(release::DATA_DIR).join("update"),
        state_dir: Path::new(release::DATA_DIR).join("state"),
        keys: PathBuf::from(KEYS),
        product: PRODUCT,
        release: release::RELEASE,
        build: release::BUILD_ID,
        control: PathBuf::from(release::CONTROL_SOCKET),
        partd: partd::proto::socket(release::RUN_DIR),
        names: &release::LOAD_SOURCE,
    };
    // It returns only when it could not start.
    let e = update::serve::run(host);
    eprintln!("the update service did not start: {e}");
    std::process::exit(1);
}

/// The trusted signers' public keys in the image.
#[cfg(target_os = "linux")]
const KEYS: &str = "/qsvs/keys/system.pub";

/// The product an image must be for: a test image's name says it is
/// one, and is updated only by a test image.
#[cfg(all(target_os = "linux", test_hooks))]
const PRODUCT: &str = release::TEST_NAME;
#[cfg(all(target_os = "linux", not(test_hooks)))]
const PRODUCT: &str = release::NAME;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsupdd is a service of a QS/VS host and runs on Linux only");
}
