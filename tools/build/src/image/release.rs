// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build keys`, `build release` and `build stream` (`docs/RELEASING.md`):
//! QS/VS's steps and files around `base/buildaid`'s release.
//!
//! A release is signed with release keys and never with a development
//! key: the release signing key (`QSVS_SIGNING_KEY`, ed25519) signs the
//! manifest, the installation payload, the update image, `SHA256SUMS`
//! and the stream's index; the Secure Boot db key (`QSVS_SB_KEY` and
//! `QSVS_SB_CERT`, RSA-2048) signs the three UKIs (Authenticode,
//! QS/400's `pesign`), before the disk and the ISO are made of them.
//!
//! `target/release/<VRM>/` holds the ISO, the update image, the
//! manifest and its signature, the notices, the release notes, the
//! catalogue (`catalogue.json`), the db certificate, `SHA256SUMS` and its
//! signature, and `stream/` with the index of this release. With
//! `--compare` (the default) the whole release is built a second time
//! from clean at the same target path, and every file must be the same.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use buildaid::release::Names;
use buildkit::Ctx;
use ed25519_dalek::SigningKey;
use tracing::{info, warn};

use crate::product::Image;

/// QS/VS's keys, certificate and stream.
pub const NAMES: Names = Names {
    prefix: "qsvs",
    sb_subject: "CN=Quartz Systems QS/VS Secure Boot db 2026,O=Quartz Systems",
    stream: "qsvs",
};

/// `build keys DIR`: a release signing key and a Secure Boot db pair,
/// made once on a trusted machine (`docs/RELEASING.md`, "The keys").
/// Nothing is written over.
pub fn keys(dir: &Path) -> Result<()> {
    buildaid::release::keys(dir, &NAMES)
}

/// The release notes of `vrm`.
fn notes(ctx: &Ctx, vrm: &str) -> PathBuf {
    ctx.paths
        .repo
        .join("docs")
        .join(format!("RELEASE_NOTES_{vrm}.md"))
}

/// What a release adds at the media's root: the certificate an operator
/// enrols, the notices and the release notes (`images/media-root`, which
/// the product's media take).
fn media_root(ctx: &Ctx, db: &pesign::DbKey, vrm: &str) -> Result<()> {
    let dir = ctx.paths.images().join("media-root");
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(NAMES.cert()), &db.cert)?;
    fs::copy(
        ctx.paths.repo.join(crate::product::NOTICES_FILE),
        dir.join("NOTICES.txt"),
    )?;
    fs::copy(notes(ctx, vrm), dir.join("RELEASE-NOTES.md"))?;
    Ok(())
}

fn copy(from: &Path, to_dir: &Path) -> Result<String> {
    let name = from
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    fs::copy(from, to_dir.join(&name))
        .with_context(|| format!("cannot copy {}", from.display()))?;
    Ok(name)
}

/// Every step of a release, once, into `target/release/<VRM>/`.
fn build_once(ctx: &Ctx, vrm: &str, key: &SigningKey, db: &pesign::DbKey) -> Result<PathBuf> {
    buildkit::notices::notices(ctx, true)?;
    buildkit::kernel::fetch::fetch(ctx)?;
    buildkit::kernel::kernel(ctx)?;
    super::userland::userland(ctx, Image::Release)?;
    buildkit::image::initramfs::initramfs(ctx)?;
    buildkit::kernel::uki(ctx, false)?;
    buildaid::release::sign_ukis(ctx, db)?;
    super::disk::disk(ctx, None)?;
    media_root(ctx, db, vrm)?;
    let iso = buildkit::image::iso::iso(ctx);
    let _ = fs::remove_dir_all(ctx.paths.images().join("media-root"));
    let iso = iso?;
    let upd = super::update::update(ctx)?;
    let manifest = super::manifest::manifest(ctx, Image::Release)?;

    let out = ctx.paths.target.join("release").join(vrm);
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    fs::create_dir_all(&out)?;
    let mut files = Vec::new();
    for from in [&iso, &upd] {
        files.push(copy(from, &out)?);
    }
    fs::copy(&manifest, out.join("MANIFEST.json"))?;
    fs::copy(
        manifest.with_file_name("manifest.sig"),
        out.join("MANIFEST.sig"),
    )?;
    fs::copy(
        ctx.paths.repo.join(crate::product::NOTICES_FILE),
        out.join("NOTICES.txt"),
    )?;
    fs::copy(notes(ctx, vrm), out.join(format!("RELEASE_NOTES_{vrm}.md")))?;
    fs::write(out.join(NAMES.cert()), &db.cert)?;
    fs::write(
        out.join("catalogue.json"),
        crate::host::catalogue::json(&ctx.product.release, &ctx.product.build_id),
    )?;
    fs::copy(
        ctx.paths.repo.join("docs").join("catalogue.schema.json"),
        out.join("catalogue.schema.json"),
    )?;
    for f in [
        "MANIFEST.json".to_string(),
        "MANIFEST.sig".into(),
        "NOTICES.txt".into(),
        format!("RELEASE_NOTES_{vrm}.md"),
        NAMES.cert(),
        "catalogue.json".into(),
        "catalogue.schema.json".into(),
    ] {
        files.push(f);
    }
    // The stream's index of this release alone; `build stream add`
    // joins it to the published one. Then the sums of every file.
    let (iso_name, upd_name) = (files[0].clone(), files[1].clone());
    buildaid::release::index_and_sums(
        &out,
        &NAMES,
        vrm,
        &ctx.product.build_id,
        crate::image::update::named_fixes(ctx)?,
        &upd_name,
        &iso_name,
        &mut files,
        key,
    )?;
    Ok(out)
}

/// `build release VERSION`: refused without release keys or with a
/// development key, from a dirty tree, or for a version not the
/// tree's; then the release, and with `compare` a second one from
/// clean, file for file the same.
pub fn release(ctx: &Ctx, version: &str, compare: bool) -> Result<PathBuf> {
    let (key, db) = buildaid::release::release_keys(ctx)?;
    let vrm = buildaid::release::check_tree(ctx, version, |named| notes(ctx, named))?;
    info!("release: {vrm} build {}", ctx.product.build_id);
    if !compare {
        let out = build_once(ctx, &vrm, &key, &db)?;
        warn!("release: not built a second time (--no-compare)");
        return Ok(out);
    }
    let again = buildaid::release::twice(ctx, || build_once(ctx, &vrm, &key, &db))?;
    for line in fs::read_to_string(again.join("SHA256SUMS"))?.lines() {
        info!("release: {line}");
    }
    Ok(again)
}

/// `build stream add RELEASE_DIR STREAM_DIR`: the release's files copied
/// into the published directory and its entry joined to the index there,
/// its sequence one further, signed again with the release signing key.
pub fn stream_add(ctx: &Ctx, release_dir: &Path, stream_dir: &Path) -> Result<()> {
    buildaid::release::stream_add(ctx, &NAMES, release_dir, stream_dir)
}

/// `build stream verify STREAM_DIR`: the index checked against the keys
/// `QSVS_TRUSTED_KEYS` names and the release key, and every file it
/// names against its size and SHA-256.
pub fn stream_verify(ctx: &Ctx, stream_dir: &Path) -> Result<stream::Index> {
    buildaid::release::stream_verify(ctx, stream_dir)
}
