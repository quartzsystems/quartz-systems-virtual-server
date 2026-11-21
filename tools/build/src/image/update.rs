// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build update`: the update image of what was built (`docs/UPDATE.md`):
//! both slots' UKIs (each carries its slot in its command line, and the
//! host installs the one of the slot it does not run) and a manifest of
//! them, of the release, its fixes and the state it reads, signed with
//! the build's key. The same inputs make the same bytes. The artifact
//! manifest lists it.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use buildkit::host::hash::sha256_bytes;
use buildkit::host::paths::read_pointer;
use buildkit::{Ctx, UkiKind};
use ed25519_dalek::Signer;
use tracing::info;
use update::manifest::{File, Manifest};

/// The pointer to the newest update image, in the images directory.
pub const POINTER: &str = "update.path";
/// The fixes the release names: `QSVS_FIXES`, comma-separated.
const FIXES: &str = "QSVS_FIXES";

/// The fixes named in `text`, each `QVnnnnn`, in order and once.
/// The fixes `QSVS_FIXES` names, checked.
pub fn named_fixes() -> Result<Vec<String>> {
    fixes(&std::env::var(FIXES).unwrap_or_default())
}

fn fixes(text: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for f in text.split(',').map(str::trim).filter(|f| !f.is_empty()) {
        if !update::manifest::valid_fix(&qsvsupdd::FORMAT, f) {
            bail!("{FIXES}: `{f}` is not a fix (QV and five digits)");
        }
        if !out.iter().any(|o| o == f) {
            out.push(f.to_string());
        }
    }
    Ok(out)
}

fn slot(ctx: &Ctx, kind: UkiKind) -> Result<Vec<u8>> {
    let path = read_pointer(&ctx.paths.uki_pointer(kind))
        .with_context(|| format!("no UKI for {kind:?}; run `uki` first"))?;
    fs::read(&path).with_context(|| format!("cannot read {}", path.display()))
}

/// `build update`: the image's path.
pub fn update(ctx: &Ctx) -> Result<PathBuf> {
    let a = slot(ctx, UkiKind::SlotA)?;
    let b = slot(ctx, UkiKind::SlotB)?;
    let entry = |name: &str, bytes: &[u8]| File {
        name: name.to_string(),
        bytes: bytes.len() as u64,
        sha256: sha256_bytes(bytes),
    };
    let manifest = Manifest {
        product: ctx.product.name.to_string(),
        release: ctx.product.release.clone(),
        build: ctx.product.build_id.clone(),
        fixes: fixes(&std::env::var(FIXES).unwrap_or_default())?,
        schemas: qsvsupdd::schemas::this_release(),
        files: vec![
            entry(update::verify::SLOT_A, &a),
            entry(update::verify::SLOT_B, &b),
        ],
    };
    let key = buildkit::image::manifest::signing_key(ctx)?;
    let image = update::make(&qsvsupdd::FORMAT, &manifest, &a, &b, |text| {
        key.sign(text).to_bytes()
    });
    let hash = sha256_bytes(&image);
    let out = ctx.paths.images();
    fs::create_dir_all(&out)?;
    let path = out.join(format!(
        "{}-{}-update-{}.upd",
        ctx.product.vendor,
        ctx.product.release,
        &hash[..12]
    ));
    fs::write(&path, &image)?;
    fs::write(out.join(POINTER), format!("{}\n", path.display()))?;
    info!(
        "update: {} ({} bytes, sha256 {hash}; {} {} build {}, fixes {})",
        path.display(),
        image.len(),
        manifest.product,
        manifest.release,
        manifest.build,
        if manifest.fixes.is_empty() {
            "none".to_string()
        } else {
            manifest.fixes.join(",")
        }
    );
    Ok(path)
}

/// The artifact manifest's JSON with the update image at `path` among
/// its artifacts: inserted as the pretty form writes one, last.
pub fn with_artifact(json: &str, path: &str, sha256: &str, size: u64) -> Result<String> {
    let end = json
        .rfind("\n  ]\n}")
        .context("the manifest's artifacts do not end as written")?;
    let entry = format!(
        ",\n    {{\n      \"path\": \"{path}\",\n      \"kind\": \"update\",\n      \"sha256\": \"{sha256}\",\n      \"size\": {size}\n    }}"
    );
    Ok(format!("{}{entry}{}", &json[..end], &json[end..]))
}

/// List the update image in the artifact manifest and sign it again,
/// as `buildkit` signs it. Nothing when no update image was built.
pub fn list(ctx: &Ctx, manifest: &std::path::Path) -> Result<()> {
    let Ok(path) = read_pointer(&ctx.paths.images().join(POINTER)) else {
        return Ok(());
    };
    if !path.is_file() {
        return Ok(());
    }
    let bytes = fs::read(&path)?;
    let json = fs::read_to_string(manifest)?;
    let json = with_artifact(
        &json,
        &ctx.paths.relative(&path),
        &sha256_bytes(&bytes),
        bytes.len() as u64,
    )?;
    fs::write(manifest, &json)?;
    let key = buildkit::image::manifest::signing_key(ctx)?;
    let signature = key.sign(json.as_bytes());
    let sig = format!(
        "{{\n  \"algorithm\": \"ed25519\",\n  \"public_key\": \"{}\",\n  \"manifest_sha256\": \"{}\",\n  \"signature\": \"{}\"\n}}\n",
        hex::encode(key.verifying_key().to_bytes()),
        sha256_bytes(json.as_bytes()),
        hex::encode(signature.to_bytes())
    );
    fs::write(manifest.with_file_name("manifest.sig"), sig)?;
    info!("manifest: the update image listed, signed again");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_are_named_once_and_checked() {
        assert_eq!(
            fixes(" QV00001, QV00002,QV00001,").unwrap(),
            ["QV00001", "QV00002"]
        );
        assert!(fixes("").unwrap().is_empty());
        assert!(fixes("PTF0001").is_err());
    }

    #[test]
    fn the_update_image_is_the_last_artifact() {
        let json = "{\n  \"product\": \"QS/VS\",\n  \"artifacts\": [\n    {\n      \"path\": \"images/a\",\n      \"kind\": \"disk\",\n      \"sha256\": \"00\",\n      \"size\": 1\n    }\n  ]\n}\n";
        let with = with_artifact(json, "images/u.upd", "ab", 9).unwrap();
        let v: serde_json::Value = serde_json::from_str(&with).unwrap();
        let all = v["artifacts"].as_array().unwrap();
        assert_eq!((all.len(), all[1]["kind"].as_str()), (2, Some("update")));
        assert_eq!(all[1]["size"], 9);
        assert!(with_artifact("{}", "x", "y", 1).is_err());
    }
}
