// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build keys`, `build release` and `build stream` (`docs/RELEASING.md`).
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

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use buildkit::host::hash::sha256_file;
use buildkit::host::paths;
use buildkit::kernel::SOURCE_DATE_EPOCH;
use buildkit::{Ctx, UkiKind};
use ed25519_dalek::{Signer, SigningKey};
use tracing::{info, warn};

use crate::product::Image;

/// The Secure Boot db key, PKCS#8, PEM or DER.
pub const SB_KEY_ENV: &str = "QSVS_SB_KEY";
/// Its certificate, DER or PEM.
pub const SB_CERT_ENV: &str = "QSVS_SB_CERT";
/// The certificate's name on the media and in the release.
pub const CERT_NAME: &str = "qsvs-db.cer";
/// The product's stream.
pub const STREAM: &str = "qsvs";
/// The release's subject for a new db key.
const SB_SUBJECT: &str = "CN=Quartz Systems QS/VS Secure Boot db 2026,O=Quartz Systems";
/// 2026-01-01T00:00:00Z, the build's fixed epoch: a db certificate's start.
const NOT_BEFORE: u64 = 1_767_225_600;

/// `build keys DIR`: a release signing key and a Secure Boot db pair,
/// made once on a trusted machine (`docs/RELEASING.md`, "The keys").
/// Nothing is written over.
pub fn keys(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    let signing = dir.join("qsvs-release.ed25519");
    let sb_key = dir.join("qsvs-db.key");
    let sb_cert = dir.join(CERT_NAME);
    for p in [&signing, &sb_key, &sb_cert] {
        ensure!(
            !p.exists(),
            "{} exists: keys are never made over",
            p.display()
        );
    }
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    write_private(&signing, &format!("{}\n", hex::encode(seed)))?;
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    // What a release that is to trust it names in QSVS_TRUSTED_KEYS.
    fs::write(
        dir.join("qsvs-release.pub"),
        format!("{}\n", hex::encode(public)),
    )?;
    let pair = pesign::keys::generate::new_pair(&pesign::keys::generate::Spec {
        subject: SB_SUBJECT,
        not_before: NOT_BEFORE,
        years: 30,
    })?;
    let der = pair
        .key
        .to_pkcs8_der()
        .map_err(|e| anyhow::anyhow!("the db key: {e}"))?;
    write_private(&sb_key, &certs::pem::encode("PRIVATE KEY", &der))?;
    fs::write(&sb_cert, &pair.cert)?;
    fs::write(dir.join("qsvs-db.esl"), pesign::keys::esl::list(&pair.cert))?;
    info!(
        "keys: release signing key {} (public {})",
        signing.display(),
        hex::encode(public)
    );
    info!(
        "keys: Secure Boot db key {}, certificate SHA-256 {}",
        sb_key.display(),
        pair.fingerprint()
    );
    Ok(())
}

fn getrandom(buf: &mut [u8]) -> Result<()> {
    use std::io::Read;
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .context("/dev/urandom")
}

#[cfg(unix)]
fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// The release keys, or why a release cannot be signed: a development
/// key is refused, and so is a key that is not given.
fn release_keys(ctx: &Ctx) -> Result<(SigningKey, pesign::DbKey)> {
    let env = ctx.product.env_name("SIGNING_KEY");
    let Some(given) = std::env::var_os(&env) else {
        bail!(
            "a release is signed with the release signing key: set {env} (docs/RELEASING.md); \
             the build's development key is refused"
        );
    };
    let dev = ctx.paths.keys().join("dev.ed25519");
    let given = PathBuf::from(given);
    if dev.is_file() {
        let same_file = fs::canonicalize(&given).ok() == fs::canonicalize(&dev).ok();
        let same_key = fs::read_to_string(&given)
            .ok()
            .map(|t| t.trim().to_string())
            == fs::read_to_string(&dev).ok().map(|t| t.trim().to_string());
        ensure!(
            !same_file && !same_key,
            "{env} names the build's development key: a release is not signed with it"
        );
    }
    let key = buildkit::image::manifest::signing_key(ctx)?;
    let (Some(k), Some(c)) = (std::env::var_os(SB_KEY_ENV), std::env::var_os(SB_CERT_ENV)) else {
        bail!(
            "a release's UKIs are signed for Secure Boot: set {SB_KEY_ENV} and {SB_CERT_ENV} \
             (docs/RELEASING.md)"
        );
    };
    let db = pesign::DbKey::load(Path::new(&k), Path::new(&c))?;
    ensure!(
        db.cert != pesign::DbKey::development()?.cert,
        "{SB_CERT_ENV} is a development certificate: a release is not signed with it"
    );
    Ok((key, db))
}

/// Sign the three UKIs in place with the db key.
fn sign_ukis(ctx: &Ctx, db: &pesign::DbKey) -> Result<()> {
    for kind in [UkiKind::SlotA, UkiKind::SlotB, UkiKind::Media] {
        let p = paths::read_pointer(&ctx.paths.uki_pointer(kind))?;
        let signed = pesign::sign(&fs::read(&p)?, db)?;
        pesign::verify(&signed, &db.cert)?;
        fs::write(&p, &signed)?;
        info!(
            "release: UKI {kind:?} signed for Secure Boot, db key SHA-256 {}",
            db.fingerprint()
        );
    }
    Ok(())
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
    fs::write(dir.join(CERT_NAME), &db.cert)?;
    fs::copy(
        ctx.paths.repo.join(crate::product::NOTICES_FILE),
        dir.join("NOTICES.txt"),
    )?;
    fs::copy(notes(ctx, vrm), dir.join("RELEASE-NOTES.md"))?;
    Ok(())
}

/// The tree is clean and the release named is `VERSION`'s.
fn check_tree(ctx: &Ctx, version: &str) -> Result<String> {
    let release: vrm::Release = ctx
        .product
        .release
        .parse()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let named = format!(
        "V{}R{}M{}",
        version_part(version, 0)?,
        version_part(version, 1)?,
        version_part(version, 2)?
    );
    ensure!(
        named == release.to_string(),
        "release {version} ({named}) is not this tree's: VERSION and QSVS_RELEASE say {release}"
    );
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .current_dir(&ctx.paths.repo)
        .output()
        .context("git status")?;
    let dirty = String::from_utf8_lossy(&status.stdout).trim().to_string();
    ensure!(
        status.status.success() && dirty.is_empty(),
        "a release is built from a clean tree; this one has:\n{dirty}"
    );
    ensure!(
        notes(ctx, &named).is_file(),
        "{} is missing",
        notes(ctx, &named).display()
    );
    Ok(named)
}

fn version_part(version: &str, i: usize) -> Result<u8> {
    version
        .split('.')
        .nth(i)
        .and_then(|p| p.parse().ok())
        .with_context(|| format!("{version} is not MAJOR.MINOR.PATCH"))
}

/// Every step of a release, once, into `target/release/<VRM>/`.
fn build_once(ctx: &Ctx, vrm: &str, key: &SigningKey, db: &pesign::DbKey) -> Result<PathBuf> {
    buildkit::notices::notices(ctx, true)?;
    buildkit::kernel::fetch::fetch(ctx)?;
    buildkit::kernel::kernel(ctx)?;
    super::userland::userland(ctx, Image::Release)?;
    buildkit::image::initramfs::initramfs(ctx)?;
    buildkit::kernel::uki(ctx, false)?;
    sign_ukis(ctx, db)?;
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
    fs::write(out.join(CERT_NAME), &db.cert)?;
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
        CERT_NAME.into(),
        "catalogue.json".into(),
        "catalogue.schema.json".into(),
    ] {
        files.push(f);
    }
    // The stream's index of this release alone; `build stream add`
    // joins it to the published one.
    let iso_name = files[0].clone();
    let upd_name = files[1].clone();
    let entry = stream::Entry {
        release: vrm.to_string(),
        build: ctx.product.build_id.clone(),
        fixes: crate::image::update::named_fixes()?,
        updates_from: updates_from(vrm),
        published: SOURCE_DATE_EPOCH.parse()?,
        update: upd_name.clone(),
        update_bytes: fs::metadata(out.join(&upd_name))?.len(),
        update_sha256: sha256_file(&out.join(&upd_name))?,
        iso: iso_name.clone(),
        iso_bytes: fs::metadata(out.join(&iso_name))?.len(),
        iso_sha256: sha256_file(&out.join(&iso_name))?,
        released: stream::date_of(SOURCE_DATE_EPOCH.parse()?),
        notes: format!("RELEASE_NOTES_{vrm}.md"),
    };
    let index = stream::Index {
        format: stream::FORMAT,
        stream: STREAM.into(),
        sequence: 1,
        generated: SOURCE_DATE_EPOCH.parse()?,
        releases: vec![entry],
    };
    fs::create_dir_all(out.join("stream"))?;
    fs::write(
        out.join("stream").join("index.toml"),
        stream::sign(&index, key),
    )?;
    files.push("stream/index.toml".into());
    files.sort();
    let mut sums = String::new();
    for f in &files {
        let _ = writeln!(sums, "{}  {f}", sha256_file(&out.join(f))?);
    }
    fs::write(out.join("SHA256SUMS"), &sums)?;
    let sig = key.sign(sums.as_bytes());
    fs::write(
        out.join("SHA256SUMS.sig"),
        format!(
            "ed25519 {}\nkey {}\n",
            hex::encode(sig.to_bytes()),
            hex::encode(key.verifying_key().to_bytes())
        ),
    )?;
    Ok(out)
}

/// The least release `vrm` updates from: the first of its version and
/// release (V1R0M1 from V1R0M0); a new release, from the last of the one
/// before it, which a release's notes name when it is not this.
fn updates_from(vrm: &str) -> String {
    match vrm.parse::<vrm::Release>() {
        Ok(r) => vrm::Release { m: 0, ..r }.to_string(),
        Err(_) => vrm.to_string(),
    }
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

/// The files of `dir`, relative, sorted.
fn listed(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d)? {
            let p = e?.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dir) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// `build release VERSION`: refused without release keys or with a
/// development key, from a dirty tree, or for a version not the
/// tree's; then the release, and with `compare` a second one from
/// clean, file for file the same.
pub fn release(ctx: &Ctx, version: &str, compare: bool) -> Result<PathBuf> {
    let (key, db) = release_keys(ctx)?;
    let vrm = check_tree(ctx, version)?;
    info!("release: {vrm} build {}", ctx.product.build_id);
    let out = build_once(ctx, &vrm, &key, &db)?;
    if !compare {
        warn!("release: not built a second time (--no-compare)");
        return Ok(out);
    }
    let first = out.with_extension("first");
    if first.exists() {
        fs::remove_dir_all(&first)?;
    }
    fs::rename(&out, &first)?;
    // From clean, at the same path: the kernel's configuration names its
    // initramfs by its place in the target.
    for d in [
        ctx.paths.target.join("kernel"),
        ctx.paths.images(),
        ctx.paths.target.join("initramfs"),
        ctx.paths.target.join(buildkit::userland::RUST_TARGET),
    ] {
        if d.exists() {
            fs::remove_dir_all(&d).with_context(|| format!("cannot clear {}", d.display()))?;
        }
    }
    let again = build_once(ctx, &vrm, &key, &db)?;
    let (a, b) = (listed(&first)?, listed(&again)?);
    ensure!(a == b, "the second build's files differ: {a:?} and {b:?}");
    let mut differ = Vec::new();
    for f in &a {
        if fs::read(first.join(f))? != fs::read(again.join(f))? {
            differ.push(f.clone());
        }
    }
    ensure!(
        differ.is_empty(),
        "the second build differs in {differ:?}: not reproducible"
    );
    fs::remove_dir_all(&first)?;
    info!(
        "release: built twice from clean: {} files the same, byte for byte",
        a.len()
    );
    for line in fs::read_to_string(again.join("SHA256SUMS"))?.lines() {
        info!("release: {line}");
    }
    Ok(again)
}

/// `build stream add RELEASE_DIR STREAM_DIR`: the release's files copied
/// into the published directory and its entry joined to the index there,
/// its sequence one further, signed again with the release signing key.
/// `build stream verify STREAM_DIR`: the index checked against the keys
/// `QSVS_TRUSTED_KEYS` names and the release key, and every file it
/// names against its size and SHA-256.
pub fn stream_add(ctx: &Ctx, release_dir: &Path, stream_dir: &Path) -> Result<()> {
    let key = buildkit::image::manifest::signing_key(ctx)?;
    let pk = key.verifying_key().to_bytes();
    let new = fs::read(release_dir.join("stream").join("index.toml"))?;
    let new = stream::verify(&new, &[pk], 0).map_err(|e| anyhow::anyhow!("{e}"))?;
    fs::create_dir_all(stream_dir)?;
    let at = stream_dir.join("index.toml");
    let mut index = if at.is_file() {
        stream::verify(&fs::read(&at)?, &[pk], 0)
            .map_err(|e| anyhow::anyhow!("{}: {e}", at.display()))?
    } else {
        stream::Index {
            format: stream::FORMAT,
            stream: STREAM.into(),
            sequence: 0,
            generated: 0,
            releases: Vec::new(),
        }
    };
    let now: u64 = SOURCE_DATE_EPOCH.parse()?;
    for e in new.releases {
        for f in [&e.update, &e.iso] {
            fs::copy(release_dir.join(f), stream_dir.join(f))?;
        }
        index = stream::with(index, e, now);
    }
    fs::write(&at, stream::sign(&index, &key))?;
    info!(
        "stream: {} release(s), sequence {}",
        index.releases.len(),
        index.sequence
    );
    Ok(())
}

/// See [`stream_add`].
pub fn stream_verify(ctx: &Ctx, stream_dir: &Path) -> Result<stream::Index> {
    let mut trusted = vec![
        buildkit::image::manifest::signing_key(ctx)?
            .verifying_key()
            .to_bytes(),
    ];
    if let Some(p) = std::env::var_os("QSVS_TRUSTED_KEYS") {
        for line in fs::read_to_string(&p)?.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let k: [u8; 32] = hex::decode(line)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("a trusted key is 32 bytes"))?;
            trusted.push(k);
        }
    }
    let index = stream::verify(&fs::read(stream_dir.join("index.toml"))?, &trusted, 0)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    for e in &index.releases {
        for (f, bytes, sum) in [
            (&e.update, e.update_bytes, &e.update_sha256),
            (&e.iso, e.iso_bytes, &e.iso_sha256),
        ] {
            let p = stream_dir.join(f);
            ensure!(p.is_file(), "{f} is named by the index and is not there");
            ensure!(fs::metadata(&p)?.len() == bytes, "{f} is not {bytes} bytes");
            ensure!(&sha256_file(&p)? == sum, "{f}'s SHA-256 is not the index's");
        }
    }
    info!(
        "stream: index of {} verified, sequence {}, {} release(s), every file whole",
        index.stream,
        index.sequence,
        index.releases.len()
    );
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_updates_from_the_first_of_its_release() {
        assert_eq!(updates_from("V1R0M1"), "V1R0M0");
        assert_eq!(updates_from("V1R0M0"), "V1R0M0");
        assert_eq!(updates_from("V1R1M3"), "V1R1M0");
    }
}
