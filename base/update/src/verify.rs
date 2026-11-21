// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Whether an update image may be installed (`docs/UPDATE.md`), checked
//! in order: it reads; its manifest is signed by a key the running
//! image trusts; its files are whole; it is this product's; it is not
//! older than what runs unless that was allowed; its release reads the
//! state as it stands. Nothing turns a check off.

use std::ops::Range;
use std::path::Path;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use vrm::Release;

use crate::image;
use crate::manifest::Manifest;

/// Slot A's UKI in an image.
pub const SLOT_A: &str = "SLOTA.EFI";
/// Slot B's.
pub const SLOT_B: &str = "SLOTB.EFI";

/// What runs, that an image is checked against.
#[derive(Clone, Debug)]
pub struct Running {
    /// Its images' format.
    pub format: &'static crate::Format,
    /// The product's name.
    pub product: String,
    /// Its release.
    pub release: Release,
    /// The trusted signers' public keys, 32 bytes each.
    pub keys: Vec<u8>,
    /// Each state file as it stands, with its schema version.
    pub state: Vec<(String, u32)>,
}

/// Why an image is refused: each is QSI's own (`QSI1101` to `QSI1105`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not signed by a key this host trusts.
    Signature,
    /// It does not read, or a hash or a size is wrong.
    Damaged(String),
    /// Another product's.
    Product {
        /// The image's.
        theirs: String,
        /// This host's.
        ours: String,
    },
    /// Older than what runs, and that was not allowed.
    Older {
        /// The image's.
        theirs: Release,
        /// What runs.
        ours: Release,
    },
    /// Its release cannot read a state file as it stands.
    State {
        /// The file.
        file: String,
        /// Its version on the host.
        found: u32,
        /// The newest the image reads.
        reads: u32,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Signature => write!(f, "it is not signed by a key this host trusts"),
            Refusal::Damaged(why) => write!(f, "it is damaged: {why}"),
            Refusal::Product { theirs, ours } => {
                write!(f, "it is for {theirs}, and this host is {ours}")
            }
            Refusal::Older { theirs, ours } => {
                write!(f, "{theirs} is older than {ours}, which runs")
            }
            Refusal::State { file, found, reads } => write!(
                f,
                "{file} is schema version {found}, and the image's release reads {reads} at most"
            ),
        }
    }
}

/// An image that verified: its manifest, and where each slot's UKI is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verified {
    /// What it is.
    pub manifest: Manifest,
    /// Slot A's UKI and slot B's, in the image's bytes.
    pub slots: [Range<usize>; 2],
}

/// Whether `signature` over `text` is by one of `keys`.
#[must_use]
pub fn signed(text: &[u8], signature: &[u8], keys: &[u8]) -> bool {
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    let (keys, _) = keys.as_chunks::<32>();
    keys.iter().any(|key| {
        VerifyingKey::from_bytes(key).is_ok_and(|key| key.verify(text, &signature).is_ok())
    })
}

/// Verify the image `bytes` against what runs.
///
/// # Errors
/// The first check it fails.
pub fn verify(bytes: &[u8], running: &Running, allow_older: bool) -> Result<Verified, Refusal> {
    let parsed = image::parse(running.format, bytes).map_err(|d| Refusal::Damaged(d.0))?;
    let text = &bytes[parsed.manifest.clone()];
    if !signed(text, &bytes[parsed.signature.clone()], &running.keys) {
        return Err(Refusal::Signature);
    }
    let text = std::str::from_utf8(text)
        .map_err(|_| Refusal::Damaged("its manifest is not text".into()))?;
    let manifest = Manifest::parse(running.format, text).map_err(Refusal::Damaged)?;
    let mut slots = Vec::new();
    for name in [SLOT_A, SLOT_B] {
        let listed = manifest
            .file(name)
            .ok_or_else(|| Refusal::Damaged(format!("its manifest names no {name}")))?;
        let (_, range) = parsed
            .files
            .iter()
            .find(|(n, _)| n == name)
            .ok_or_else(|| Refusal::Damaged(format!("it holds no {name}")))?;
        let data = &bytes[range.clone()];
        if data.len() as u64 != listed.bytes {
            return Err(Refusal::Damaged(format!(
                "{name} is not the size its manifest says"
            )));
        }
        if hex::encode(Sha256::digest(data)) != listed.sha256 {
            return Err(Refusal::Damaged(format!(
                "{name}'s hash is not its manifest's"
            )));
        }
        slots.push(range.clone());
    }
    if manifest.product != running.product {
        return Err(Refusal::Product {
            theirs: manifest.product.clone(),
            ours: running.product.clone(),
        });
    }
    let theirs: Release = manifest
        .release
        .parse()
        .map_err(|_| Refusal::Damaged(format!("`{}` is not a release", manifest.release)))?;
    if theirs < running.release && !allow_older {
        return Err(Refusal::Older {
            theirs,
            ours: running.release,
        });
    }
    for (file, found) in &running.state {
        if let Some((_, reads)) = manifest.schemas.iter().find(|(f, _)| f == file)
            && found > reads
        {
            return Err(Refusal::State {
                file: file.clone(),
                found: *found,
                reads: *reads,
            });
        }
    }
    let [a, b]: [Range<usize>; 2] = slots
        .try_into()
        .map_err(|_| Refusal::Damaged("its slots".into()))?;
    Ok(Verified {
        manifest,
        slots: [a, b],
    })
}

/// Each state file under `dir` with its schema version, the highest of
/// a name found more than once (a definition for each partition).
#[must_use]
pub fn state_versions(dir: &Path) -> Vec<(String, u32)> {
    fn walk(dir: &Path, depth: usize, out: &mut std::collections::BTreeMap<String, u32>) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for e in read.flatten() {
            let path = e.path();
            if path.is_dir() && depth > 0 {
                walk(&path, depth - 1, out);
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("toml"))
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let version = text.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == state::store::SCHEMA_KEY).then(|| v.trim().parse::<u32>().ok())?
            });
            if let Some(v) = version {
                let e = out.entry(name).or_insert(0);
                *e = (*e).max(v);
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, 4, &mut out);
    out.into_iter().collect()
}

#[cfg(test)]
mod tests;
