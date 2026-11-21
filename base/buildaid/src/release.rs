// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a release build of any appliance does around its own steps:
//! the release keys made once, a development key refused, the UKIs
//! signed for Secure Boot, the release directory's `SHA256SUMS` signed,
//! the stream's index of the release written and signed (and joined to
//! the published one, and verified), the tree checked clean, and the
//! whole built a second time from clean and compared file for file.
//! The product gives its [`Names`]; the environment's names are its
//! `Product`'s (`<PREFIX>_SIGNING_KEY`, `_SB_KEY`, `_SB_CERT`,
//! `_TRUSTED_KEYS`).

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
use tracing::info;

/// What a product names its keys, its certificate and its stream.
#[derive(Clone, Copy, Debug)]
pub struct Names {
    /// The files' prefix: `<prefix>-release.ed25519`, `<prefix>-db.cer`.
    pub prefix: &'static str,
    /// A new db certificate's subject.
    pub sb_subject: &'static str,
    /// The stream's name: the product's, lowercase.
    pub stream: &'static str,
}

impl Names {
    /// The db certificate's file name, on the media and in a release.
    #[must_use]
    pub fn cert(&self) -> String {
        format!("{}-db.cer", self.prefix)
    }
}

/// 2026-01-01T00:00:00Z, the build's fixed epoch: a db certificate's start.
const NOT_BEFORE: u64 = 1_767_225_600;

/// A release signing key and a Secure Boot db pair in `dir`, made once
/// on a trusted machine. Nothing is written over.
///
/// # Errors
/// A key's file is there already, or could not be written.
pub fn keys(dir: &Path, names: &Names) -> Result<()> {
    fs::create_dir_all(dir)?;
    let p = names.prefix;
    let signing = dir.join(format!("{p}-release.ed25519"));
    let sb_key = dir.join(format!("{p}-db.key"));
    let sb_cert = dir.join(names.cert());
    for f in [&signing, &sb_key, &sb_cert] {
        ensure!(
            !f.exists(),
            "{} exists: keys are never made over",
            f.display()
        );
    }
    let mut seed = [0u8; 32];
    getrandom(&mut seed)?;
    write_private(&signing, &format!("{}\n", hex::encode(seed)))?;
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    // What a release that is to trust it names in its trusted keys.
    fs::write(
        dir.join(format!("{p}-release.pub")),
        format!("{}\n", hex::encode(public)),
    )?;
    let pair = pesign::keys::generate::new_pair(&pesign::keys::generate::Spec {
        subject: names.sb_subject,
        not_before: NOT_BEFORE,
        years: 30,
    })?;
    let der = pair
        .key
        .to_pkcs8_der()
        .map_err(|e| anyhow::anyhow!("the db key: {e}"))?;
    write_private(&sb_key, &certs::pem::encode("PRIVATE KEY", &der))?;
    fs::write(&sb_cert, &pair.cert)?;
    fs::write(
        dir.join(format!("{p}-db.esl")),
        pesign::keys::esl::list(&pair.cert),
    )?;
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
///
/// # Errors
/// As said.
pub fn release_keys(ctx: &Ctx) -> Result<(SigningKey, pesign::DbKey)> {
    let env = ctx.product.env_name("SIGNING_KEY");
    let Some(given) = std::env::var_os(&env) else {
        bail!(
            "a release is signed with the release signing key: set {env}; the build's \
             development key is refused"
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
    let (sb_key, sb_cert) = (
        ctx.product.env_name("SB_KEY"),
        ctx.product.env_name("SB_CERT"),
    );
    let (Some(k), Some(c)) = (std::env::var_os(&sb_key), std::env::var_os(&sb_cert)) else {
        bail!("a release's UKIs are signed for Secure Boot: set {sb_key} and {sb_cert}");
    };
    let db = pesign::DbKey::load(Path::new(&k), Path::new(&c))?;
    ensure!(
        db.cert != pesign::DbKey::development()?.cert,
        "{sb_cert} is a development certificate: a release is not signed with it"
    );
    Ok((key, db))
}

/// Sign the three UKIs in place with the db key.
///
/// # Errors
/// A UKI could not be read, signed or written.
pub fn sign_ukis(ctx: &Ctx, db: &pesign::DbKey) -> Result<()> {
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

/// The tree is clean, the release named is `VERSION`'s (`MAJOR.MINOR.PATCH`,
/// against the product's `VxRyMz`), and its notes are at `notes`: the
/// release's `VxRyMz`.
///
/// # Errors
/// As said.
pub fn check_tree(ctx: &Ctx, version: &str, notes: impl Fn(&str) -> PathBuf) -> Result<String> {
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
        "release {version} ({named}) is not this tree's: its VERSION says {release}"
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
        notes(&named).is_file(),
        "{} is missing",
        notes(&named).display()
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

/// The least release `vrm` updates from: the first of its version and
/// release (V1R0M1 from V1R0M0).
#[must_use]
pub fn updates_from(vrm: &str) -> String {
    match vrm.parse::<vrm::Release>() {
        Ok(r) => vrm::Release { m: 0, ..r }.to_string(),
        Err(_) => vrm.to_string(),
    }
}

/// The files of `dir`, relative, sorted.
///
/// # Errors
/// A directory could not be read.
pub fn listed(dir: &Path) -> Result<Vec<String>> {
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

/// The release of `release`/`build` into `out`, whose `update` and
/// `iso` are there already: its entry in a stream's index of this
/// release alone at `out/stream/index.toml`, signed; then `SHA256SUMS`
/// of `listed` (and the index) and its signature `SHA256SUMS.sig`.
///
/// # Errors
/// A file could not be read or written.
#[allow(clippy::too_many_arguments)] // a release's every fact
pub fn index_and_sums(
    out: &Path,
    names: &Names,
    release: &str,
    build: &str,
    fixes: Vec<String>,
    update: &str,
    iso: &str,
    listed: &mut Vec<String>,
    key: &SigningKey,
) -> Result<()> {
    let epoch: u64 = SOURCE_DATE_EPOCH.parse()?;
    let entry = stream::Entry {
        release: release.to_string(),
        build: build.to_string(),
        fixes,
        updates_from: updates_from(release),
        published: epoch,
        update: update.to_string(),
        update_bytes: fs::metadata(out.join(update))?.len(),
        update_sha256: sha256_file(&out.join(update))?,
        iso: iso.to_string(),
        iso_bytes: fs::metadata(out.join(iso))?.len(),
        iso_sha256: sha256_file(&out.join(iso))?,
        released: stream::date_of(epoch),
        notes: format!("RELEASE_NOTES_{release}.md"),
    };
    let index = stream::Index {
        format: stream::FORMAT,
        stream: names.stream.into(),
        sequence: 1,
        generated: epoch,
        releases: vec![entry],
    };
    fs::create_dir_all(out.join("stream"))?;
    fs::write(
        out.join("stream").join("index.toml"),
        stream::sign(&index, key),
    )?;
    listed.push("stream/index.toml".into());
    listed.sort();
    let mut sums = String::new();
    for f in listed.iter() {
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
    Ok(())
}

/// A release built by `once`, then built again from clean at the same
/// target path (the kernel's configuration names its initramfs by its
/// place), and every file of the two compared: the second one's
/// directory.
///
/// # Errors
/// A build failed, or the two differ.
pub fn twice(ctx: &Ctx, mut once: impl FnMut() -> Result<PathBuf>) -> Result<PathBuf> {
    let out = once()?;
    let first = out.with_extension("first");
    if first.exists() {
        fs::remove_dir_all(&first)?;
    }
    fs::rename(&out, &first)?;
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
    let again = once()?;
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
    Ok(again)
}

/// The release's files copied into the published `stream_dir` and its
/// entry joined to the index there, its sequence one further, signed
/// again with the release signing key.
///
/// # Errors
/// An index does not verify, or a file could not be copied.
pub fn stream_add(ctx: &Ctx, names: &Names, release_dir: &Path, stream_dir: &Path) -> Result<()> {
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
            stream: names.stream.into(),
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
        if !e.notes.is_empty() && release_dir.join(&e.notes).is_file() {
            fs::copy(release_dir.join(&e.notes), stream_dir.join(&e.notes))?;
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

/// The index of `stream_dir` checked against the release key and the
/// keys `<PREFIX>_TRUSTED_KEYS` names, and every file it names against
/// its size and SHA-256.
///
/// # Errors
/// As said.
pub fn stream_verify(ctx: &Ctx, stream_dir: &Path) -> Result<stream::Index> {
    let mut trusted = vec![
        buildkit::image::manifest::signing_key(ctx)?
            .verifying_key()
            .to_bytes(),
    ];
    if let Some(p) = std::env::var_os(ctx.product.env_name("TRUSTED_KEYS")) {
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

    #[test]
    fn a_files_list_is_relative_and_sorted() {
        let d = std::env::temp_dir().join(format!("buildaid-listed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("stream")).unwrap();
        fs::write(d.join("b"), "").unwrap();
        fs::write(d.join("stream").join("index.toml"), "").unwrap();
        fs::write(d.join("a"), "").unwrap();
        assert_eq!(listed(&d).unwrap(), ["a", "b", "stream/index.toml"]);
        fs::remove_dir_all(&d).unwrap();
    }
}
