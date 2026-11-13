// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The initramfs of every UKI: `/init` (qsvsinit), the mount points, a
//! console node, and the trusted signer keys under `/qsvs/keys/`. A
//! slot's has the services and the VMM under `/qsvs/bin/`, and
//! `qsvshook` in a test image; the media's has the installer there
//! instead.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use buildkit::image::cpio::Entry;
use buildkit::image::initramfs::binary;
use buildkit::{Ctx, UkiKind};
use tracing::info;

/// A file of further signers the image trusts: ed25519 public keys, 64
/// hex digits each, one per line (`#` comments).
const TRUSTED_ENV: &str = "QSVS_TRUSTED_KEYS";

/// Bytes of an ed25519 public key.
const KEY_LEN: usize = 32;

/// The entries of a UKI of `kind`. The kernel does not mount devtmpfs
/// for an initramfs, so a static console node lets it open the first
/// console. `/data` is where `QSVSDATA` is mounted, and holds the state
/// in memory when it is not; `/etc` is for the resolver's file.
fn all(ctx: &Ctx, hooks: bool, kind: UkiKind) -> Result<Vec<Entry>> {
    let mut entries = vec![
        Entry::dir("data"),
        Entry::dir("dev"),
        Entry::chardev("dev/console", 5, 1),
        Entry::dir("etc"),
        Entry::file("init", binary(ctx, super::INIT)?),
        Entry::dir("proc"),
        Entry::dir("qsvs"),
        Entry::dir("qsvs/bin"),
        Entry::dir("qsvs/keys"),
        Entry::file("qsvs/keys/system.pub", trusted_keys(ctx)?),
        Entry::dir("run"),
        Entry::dir("sys"),
    ];
    // The media runs the installer and nothing else.
    let binaries: Vec<&str> = if kind == UkiKind::Media {
        vec![super::INSTALLER]
    } else {
        let hook = hooks.then_some(super::HOOK);
        super::SERVICES
            .into_iter()
            .chain([super::VMM])
            .chain(hook)
            .collect()
    };
    for name in binaries {
        entries.push(Entry::file(&format!("qsvs/bin/{name}"), binary(ctx, name)?));
    }
    Ok(entries)
}

/// The release image's entries.
pub fn entries(ctx: &Ctx, kind: UkiKind) -> Result<Vec<Entry>> {
    all(ctx, false, kind)
}

/// The test image's entries: the release image's and the test hooks.
pub fn test_entries(ctx: &Ctx, kind: UkiKind) -> Result<Vec<Entry>> {
    all(ctx, true, kind)
}

/// The keys in `text`, appended to `out` unless already there.
fn add_keys(out: &mut Vec<u8>, text: &str) -> Result<()> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    for line in lines {
        let key = hex::decode(line).with_context(|| format!("{line:?} is not hex"))?;
        ensure!(key.len() == KEY_LEN, "{line:?} is not a 32-byte key");
        if !out.chunks(KEY_LEN).any(|have| have == key) {
            out.extend_from_slice(&key);
        }
    }
    Ok(())
}

/// `/qsvs/keys/system.pub`: the build key's public key, then each key
/// `QSVS_TRUSTED_KEYS` lists; 32 bytes each.
fn trusted_keys(ctx: &Ctx) -> Result<Vec<u8>> {
    let mut out = buildkit::image::manifest::signing_key(ctx)?
        .verifying_key()
        .to_bytes()
        .to_vec();
    if let Some(path) = std::env::var_os(TRUSTED_ENV) {
        let path = PathBuf::from(path);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("cannot read {TRUSTED_ENV}={}", path.display()))?;
        add_keys(&mut out, &text).with_context(|| TRUSTED_ENV.to_string())?;
    }
    info!("initramfs: {} trusted signer(s)", out.len() / KEY_LEN);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_read_once_each_and_checked() {
        let a = "11".repeat(32);
        let b = "22".repeat(32);
        let mut out = vec![0x11; 32];
        add_keys(&mut out, &format!("# signers\n{a}\n\n  {b}\n{b}\n")).unwrap();
        assert_eq!(out.len(), 64);
        assert_eq!(&out[32..], [0x22; 32]);
        assert!(add_keys(&mut out, "zz").is_err());
        assert!(add_keys(&mut out, "1122").is_err());
    }
}
