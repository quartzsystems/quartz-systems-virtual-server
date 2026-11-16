// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Code update (`docs/UPDATE.md`): an update image staged on `/data`,
//! verified against the running image's trusted keys, installed into
//! the slot that is not running on every load-source disk, marked
//! pending, and activated by a restart into it.
//!
//! - [`manifest`]: what an image is, as signed text.
//! - [`image`]: the image's container, read as hostile.
//! - [`verify`]: the checks an image passes before it is installed.
//! - [`schemas`]: the state files this release reads, and their versions.
//! - [`proto`]: the update service's socket.
//! - `serve` (Linux): the service: staging, the install, the slots.

#![forbid(unsafe_code)]

pub mod image;
pub mod manifest;
pub mod proto;
pub mod schemas;
#[cfg(target_os = "linux")]
pub mod serve;
pub mod verify;

pub use manifest::Manifest;
pub use verify::{Refusal, Running, Verified};

/// The image of `manifest` holding `slot_a` and `slot_b`, its manifest
/// signed by `sign`.
pub fn make(
    manifest: &Manifest,
    slot_a: &[u8],
    slot_b: &[u8],
    sign: impl Fn(&[u8]) -> [u8; image::SIGNATURE],
) -> Vec<u8> {
    let text = manifest.render();
    let signature = sign(text.as_bytes());
    image::write(
        &text,
        &signature,
        &[(verify::SLOT_A, slot_a), (verify::SLOT_B, slot_b)],
    )
}

/// One input of the fuzzer: the image's parser, its manifest's and the
/// verification, each read as hostile.
pub fn fuzz_image(data: &[u8]) {
    if let Ok(p) = image::parse(data) {
        let _ = std::str::from_utf8(&data[p.manifest.clone()]).map(Manifest::parse);
    }
    let _ = verify::verify(
        data,
        &Running {
            product: "QS/VS".into(),
            release: vrm::Release { v: 1, r: 0, m: 0 },
            keys: vec![0x42; 32],
            state: vec![("host.toml".into(), 1)],
        },
        false,
    );
}

/// One input of the fuzzer: a manifest's text alone.
pub fn fuzz_manifest(data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(m) = Manifest::parse(text)
    {
        // What reads renders to what reads the same.
        assert_eq!(Manifest::parse(&m.render()).as_ref(), Ok(&m));
    }
}
