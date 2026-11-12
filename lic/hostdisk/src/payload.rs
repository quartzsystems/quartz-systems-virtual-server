// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the installation media carries beside its own boot image: the
//! two slots' UKIs and a manifest of them, signed by the build's key.
//! The build writes it; the installer checks the signature against the
//! keys in its own initramfs, and every file against the manifest,
//! before it writes anything.
//!
//! The manifest is text, a word and its values a line:
//!
//! ```text
//! QSVS-PAYLOAD 1
//! product QS/VS
//! release V1R0M0
//! build a1b2c3d
//! file SLOTA.EFI 12837888 7824c4c3...
//! file SLOTB.EFI 12837888 531a6a4f...
//! ```
//!
//! This is not the artifact manifest of a build (`manifest.json`):
//! that one lists the ISO, so it cannot be on it.

use std::fmt::Write as _;

/// The directory of the payload on the media.
pub const DIR: &str = "QSVS";
/// Slot A's UKI in [`DIR`].
pub const SLOT_A: &str = "SLOTA.EFI";
/// Slot B's UKI in [`DIR`].
pub const SLOT_B: &str = "SLOTB.EFI";
/// The manifest in [`DIR`].
pub const MANIFEST: &str = "PAYLOAD.TXT";
/// The manifest's signature in [`DIR`]: ed25519, 128 hex digits.
pub const SIGNATURE: &str = "PAYLOAD.SIG";

/// The manifest's first line.
const HEAD: &str = "QSVS-PAYLOAD 1";

/// The ISO volume identifier of a product's media at a release.
pub fn volume_id(vendor: &str, release: &str) -> String {
    format!("{vendor}_{release}")
}

/// One file of the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Its name in [`DIR`].
    pub name: String,
    /// Its length.
    pub bytes: u64,
    /// Its SHA-256, 64 lower-case hex digits.
    pub sha256: String,
}

/// The manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The product's name; a test image's says that it is one.
    pub product: String,
    /// The release.
    pub release: String,
    /// The build ID.
    pub build: String,
    /// The files.
    pub files: Vec<File>,
}

impl Manifest {
    /// The manifest's text: what is signed.
    pub fn render(&self) -> String {
        let mut out = format!(
            "{HEAD}\nproduct {}\nrelease {}\nbuild {}\n",
            self.product, self.release, self.build
        );
        for file in &self.files {
            let _ = writeln!(out, "file {} {} {}", file.name, file.bytes, file.sha256);
        }
        out
    }

    /// The manifest of `text`, or why it is not one.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        if lines.next() != Some(HEAD) {
            return Err(format!("the manifest does not start with `{HEAD}`"));
        }
        let mut manifest = Manifest {
            product: String::new(),
            release: String::new(),
            build: String::new(),
            files: Vec::new(),
        };
        for line in lines {
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            match word {
                "product" => manifest.product = rest.to_string(),
                "release" => manifest.release = rest.to_string(),
                "build" => manifest.build = rest.to_string(),
                "file" => {
                    let parts: Vec<&str> = rest.split(' ').collect();
                    let file = match parts.as_slice() {
                        [name, bytes, sha256]
                            if sha256.len() == 64
                                && sha256.bytes().all(|b| b.is_ascii_hexdigit()) =>
                        {
                            bytes.parse().ok().map(|bytes| File {
                                name: (*name).to_string(),
                                bytes,
                                sha256: sha256.to_ascii_lowercase(),
                            })
                        }
                        _ => None,
                    };
                    manifest.files.push(
                        file.ok_or_else(|| format!("`{line}` is not a file of the manifest"))?,
                    );
                }
                _ => return Err(format!("`{line}` is not a line of the manifest")),
            }
        }
        if manifest.product.is_empty() || manifest.release.is_empty() {
            return Err("the manifest names no product or no release".into());
        }
        Ok(manifest)
    }

    /// The file named `name`.
    pub fn file(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|f| f.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            product: "QS/VS (test image)".into(),
            release: "V1R0M0".into(),
            build: "a1b2c3d".into(),
            files: vec![
                File {
                    name: SLOT_A.into(),
                    bytes: 12_837_888,
                    sha256: "7824c4c3".repeat(8),
                },
                File {
                    name: SLOT_B.into(),
                    bytes: 12_837_888,
                    sha256: "531a6a4f".repeat(8),
                },
            ],
        }
    }

    #[test]
    fn the_manifest_round_trips() {
        let m = manifest();
        let text = m.render();
        assert!(text.starts_with("QSVS-PAYLOAD 1\nproduct QS/VS (test image)\n"));
        assert_eq!(Manifest::parse(&text), Ok(m.clone()));
        assert_eq!(m.file(SLOT_B).unwrap().sha256.len(), 64);
        assert!(m.file("OTHER").is_none());
        assert_eq!(volume_id("QSVS", "V1R0M0"), "QSVS_V1R0M0");
    }

    #[test]
    fn what_is_not_a_manifest_is_refused() {
        let text = manifest().render();
        for bad in [
            text.replacen("QSVS-PAYLOAD 1", "QSVS-PAYLOAD 2", 1),
            text.replacen("file SLOTA.EFI 12837888", "file SLOTA.EFI many", 1),
            format!("{text}extra line\n"),
            text.replacen("product QS/VS (test image)\n", "", 1),
            text.replacen(&"7824c4c3".repeat(8), "abc", 1),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "{bad}");
        }
    }
}
