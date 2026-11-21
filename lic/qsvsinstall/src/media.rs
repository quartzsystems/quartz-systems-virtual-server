// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The installation media of this release: `base/loadsource::payload`'s
//! search, mount and check, with QS/VS's keys, mount point and release.
//! The manifest must be signed by a key of this initramfs and every
//! file must have the manifest's length and hash; media that fails
//! either is refused before anything is written.

#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::time::Duration;

pub use loadsource::payload::Payload;

/// The trusted signers' public keys, 32 bytes each.
pub const KEYS: &str = "/qsvs/keys/system.pub";
/// Where the media is mounted.
pub const MOUNT: &str = "/run/media";
/// How long the media's device may take to appear.
const WAIT: Duration = Duration::from_secs(20);

/// Find, mount and verify the media: its device and its payload.
#[cfg(target_os = "linux")]
pub fn open() -> Result<(PathBuf, Payload), String> {
    loadsource::payload::open(
        &release::LOAD_SOURCE,
        &release::RELEASE.to_string(),
        std::path::Path::new(KEYS),
        MOUNT,
        WAIT,
    )
}
