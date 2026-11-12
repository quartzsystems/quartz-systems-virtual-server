// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The installation media: the boot image with the media UKI, the same
//! UKI loose in the tree, and the payload the installer writes a load
//! source from: the two slots' UKIs and a manifest of them, signed
//! with the build's key (`hostdisk::payload`). The installer's
//! initramfs holds the public key, so media and installer of one build
//! trust each other and nothing else does.

use std::fs;

use anyhow::{Context, Result};
use buildkit::host::hash::sha256_bytes;
use buildkit::host::paths::read_pointer;
use buildkit::image::iso::{Item, Media};
use buildkit::{Ctx, UkiKind};
use ed25519_dalek::Signer;
use hostdisk::payload::{self, File, Manifest};
use tracing::info;

/// A slot's UKI and its entry in the manifest.
fn slot(ctx: &Ctx, kind: UkiKind, name: &str) -> Result<(Vec<u8>, File)> {
    let path = read_pointer(&ctx.paths.uki_pointer(kind))
        .with_context(|| format!("no UKI for {name}; run `uki` first"))?;
    let bytes = fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let entry = File {
        name: name.to_string(),
        bytes: bytes.len() as u64,
        sha256: sha256_bytes(&bytes),
    };
    Ok((bytes, entry))
}

/// What goes on the media around the media UKI.
pub fn media(ctx: &Ctx, _uki: &[u8]) -> Result<Media> {
    let (slot_a, entry_a) = slot(ctx, UkiKind::SlotA, payload::SLOT_A)?;
    let (slot_b, entry_b) = slot(ctx, UkiKind::SlotB, payload::SLOT_B)?;
    let manifest = Manifest {
        product: ctx.product.name.to_string(),
        release: ctx.product.release.clone(),
        build: ctx.product.build_id.clone(),
        files: vec![entry_a, entry_b],
    };
    let text = manifest.render();
    let key = buildkit::image::manifest::signing_key(ctx)?;
    let signature = hex::encode(key.sign(text.as_bytes()).to_bytes());
    info!(
        "iso: payload of {} {}: both slots, manifest signed by {}",
        manifest.product,
        manifest.release,
        hex::encode(key.verifying_key().to_bytes())
    );
    let file = |name: &str, bytes: Vec<u8>| Item::File(format!("{}/{name}", payload::DIR), bytes);
    Ok(Media {
        volume_id: payload::volume_id(ctx.product.vendor, &ctx.product.release),
        tree: vec![
            Item::Boot,
            file(payload::MANIFEST, text.into_bytes()),
            file(payload::SIGNATURE, format!("{signature}\n").into_bytes()),
            file(payload::SLOT_A, slot_a),
            file(payload::SLOT_B, slot_b),
        ],
        boot_files: Vec::new(),
        uki_copies: Vec::new(),
    })
}
