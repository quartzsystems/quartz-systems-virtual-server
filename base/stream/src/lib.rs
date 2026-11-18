// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A product stream's index (`docs/RELEASING.md`, "The stream"): every
//! release of a product, each with its build, the fixes it names, the
//! least release it updates from, and its update image's and ISO's
//! names, sizes and SHA-256, published beside those files.
//!
//! The form is QS/400's signed catalog's (its `docs/UPDATE_PROTOCOL.md`
//! section 3): UTF-8 TOML in a fixed order, then one line
//! `# signature <128 hex digits>`, the ed25519 signature of every byte
//! before it. A reader verifies the bytes as received against the keys
//! it trusts and parses only what verified; `sequence` only grows, and a
//! reader that has seen one refuses a lower one.
//!
//! A host fetches nothing: a console reads the index ([`newest_for`])
//! to offer "update available", downloads the update image and brings
//! it to the host, which verifies the image itself.

#![forbid(unsafe_code)]

use std::fmt::Write as _;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::Deserialize;
use vrm::Release;

/// The form this crate writes and reads.
pub const FORMAT: u32 = 1;
/// The signature line's start.
const SIGNATURE: &str = "# signature ";

/// One release of the stream.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Entry {
    /// `VxRyMz`.
    pub release: String,
    /// Its build ID.
    pub build: String,
    /// The fixes it names.
    pub fixes: Vec<String>,
    /// The least release it updates from: a host older than this
    /// installs it from media instead.
    pub updates_from: String,
    /// When it was published, Unix seconds.
    pub published: u64,
    /// Its update image's file name, beside the index.
    pub update: String,
    /// The update image's bytes.
    pub update_bytes: u64,
    /// The update image's SHA-256, lowercase hex.
    pub update_sha256: String,
    /// Its installation ISO's file name, beside the index.
    pub iso: String,
    /// The ISO's bytes.
    pub iso_bytes: u64,
    /// The ISO's SHA-256, lowercase hex.
    pub iso_sha256: String,
}

impl Entry {
    /// Its release, parsed.
    pub fn parsed(&self) -> Option<Release> {
        self.release.parse().ok()
    }
}

/// The index.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Index {
    /// [`FORMAT`].
    pub format: u32,
    /// The stream's name: the product's, lowercase.
    pub stream: String,
    /// Grows with each index published.
    pub sequence: u64,
    /// When it was written, Unix seconds.
    pub generated: u64,
    /// Every release, oldest first.
    #[serde(default, rename = "release")]
    pub releases: Vec<Entry>,
}

/// Why an index was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// No signature line, or not one of hex.
    Unsigned,
    /// No trusted key signed it.
    NotTrusted,
    /// It is not an index of this form.
    Form(String),
    /// Older than one already seen.
    Older {
        /// Its sequence.
        sequence: u64,
        /// The highest seen.
        seen: u64,
    },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Unsigned => f.write_str("the index is not signed"),
            Refused::NotTrusted => f.write_str("no trusted key signed the index"),
            Refused::Form(why) => write!(f, "not an index: {why}"),
            Refused::Older { sequence, seen } => write!(
                f,
                "the index's sequence {sequence} is older than {seen}, already seen"
            ),
        }
    }
}

fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The index's canonical bytes, before the signature: keys in a fixed
/// order, releases oldest first.
pub fn render(index: &Index) -> String {
    let mut releases = index.releases.clone();
    releases.sort_by_key(|e| (e.parsed(), e.release.clone()));
    let mut out = String::new();
    let _ = writeln!(out, "format = {}", index.format);
    let _ = writeln!(out, "stream = {}", quoted(&index.stream));
    let _ = writeln!(out, "sequence = {}", index.sequence);
    let _ = writeln!(out, "generated = {}", index.generated);
    for e in &releases {
        let fixes: Vec<String> = e.fixes.iter().map(|f| quoted(f)).collect();
        let _ = writeln!(out, "\n[[release]]");
        let _ = writeln!(out, "release = {}", quoted(&e.release));
        let _ = writeln!(out, "build = {}", quoted(&e.build));
        let _ = writeln!(out, "fixes = [{}]", fixes.join(", "));
        let _ = writeln!(out, "updates_from = {}", quoted(&e.updates_from));
        let _ = writeln!(out, "published = {}", e.published);
        let _ = writeln!(out, "update = {}", quoted(&e.update));
        let _ = writeln!(out, "update_bytes = {}", e.update_bytes);
        let _ = writeln!(out, "update_sha256 = {}", quoted(&e.update_sha256));
        let _ = writeln!(out, "iso = {}", quoted(&e.iso));
        let _ = writeln!(out, "iso_bytes = {}", e.iso_bytes);
        let _ = writeln!(out, "iso_sha256 = {}", quoted(&e.iso_sha256));
    }
    out
}

/// The index signed with `key`: its canonical bytes and the signature
/// line.
pub fn sign(index: &Index, key: &SigningKey) -> String {
    let body = render(index);
    let signature = key.sign(body.as_bytes());
    format!("{body}{SIGNATURE}{}\n", hex::encode(signature.to_bytes()))
}

/// The index in `bytes`, when one of `trusted` signed it and its
/// sequence is not below `seen`. Only the bytes that verified are read.
pub fn verify(bytes: &[u8], trusted: &[[u8; 32]], seen: u64) -> Result<Index, Refused> {
    let text = std::str::from_utf8(bytes).map_err(|_| Refused::Form("not UTF-8".into()))?;
    let at = text.rfind(SIGNATURE).ok_or(Refused::Unsigned)?;
    let (body, line) = text.split_at(at);
    if !body.is_empty() && !body.ends_with('\n') {
        return Err(Refused::Unsigned);
    }
    let hex_sig = line[SIGNATURE.len()..].trim_end_matches('\n');
    let raw: [u8; 64] = hex::decode(hex_sig)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or(Refused::Unsigned)?;
    let signature = Signature::from_bytes(&raw);
    let trustworthy = trusted.iter().any(|k| {
        VerifyingKey::from_bytes(k).is_ok_and(|v| v.verify(body.as_bytes(), &signature).is_ok())
    });
    if !trustworthy {
        return Err(Refused::NotTrusted);
    }
    let index: Index = toml::from_str(body).map_err(|e| Refused::Form(e.to_string()))?;
    if index.format != FORMAT {
        return Err(Refused::Form(format!("format {}", index.format)));
    }
    if let Some(e) = index.releases.iter().find(|e| e.parsed().is_none()) {
        return Err(Refused::Form(format!("release {:?}", e.release)));
    }
    if index.sequence < seen {
        return Err(Refused::Older {
            sequence: index.sequence,
            seen,
        });
    }
    Ok(index)
}

/// The update a host at `running` is offered: the newest release past
/// it that updates from it. `None` when it is current, or when every
/// newer release wants a later starting point (it is then installed
/// from media, or updated in steps).
pub fn newest_for(index: &Index, running: Release) -> Option<&Entry> {
    index
        .releases
        .iter()
        .filter(|e| e.parsed().is_some_and(|r| r > running))
        .filter(|e| {
            e.updates_from
                .parse::<Release>()
                .is_ok_and(|f| f <= running)
        })
        .max_by_key(|e| e.parsed())
}

/// `index` with `entry` in it, in place of an entry of the same release,
/// and its sequence one further.
pub fn with(mut index: Index, entry: Entry, now: u64) -> Index {
    index.releases.retain(|e| e.release != entry.release);
    index.releases.push(entry);
    index.sequence += 1;
    index.generated = now;
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(release: &str, from: &str) -> Entry {
        Entry {
            release: release.into(),
            build: format!("b-{release}"),
            fixes: vec!["QV00001".into()],
            updates_from: from.into(),
            published: 1_793_000_000,
            update: format!("X-{release}-update.upd"),
            update_bytes: 100,
            update_sha256: "ab".repeat(32),
            iso: format!("X-{release}.iso"),
            iso_bytes: 200,
            iso_sha256: "cd".repeat(32),
        }
    }

    fn index() -> Index {
        Index {
            format: FORMAT,
            stream: "product".into(),
            sequence: 3,
            generated: 1_793_000_100,
            releases: vec![entry("V1R0M1", "V1R0M0"), entry("V1R0M0", "V1R0M0")],
        }
    }

    #[test]
    fn a_signed_index_verifies_and_reads_back_as_written() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let text = sign(&index(), &key);
        let pk = key.verifying_key().to_bytes();
        let back = verify(text.as_bytes(), &[[1; 32], pk], 0).unwrap();
        assert_eq!(render(&back), render(&index()));
        // Canonical: oldest first, whatever order it was given in.
        assert!(text.find("V1R0M0\"").unwrap() < text.find("V1R0M1\"").unwrap());
        assert_eq!(sign(&back, &key), text);
    }

    #[test]
    fn a_changed_byte_another_key_or_an_older_sequence_is_refused() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let pk = key.verifying_key().to_bytes();
        let text = sign(&index(), &key);
        let changed = text.replacen("update_bytes = 100", "update_bytes = 101", 1);
        assert_eq!(
            verify(changed.as_bytes(), &[pk], 0),
            Err(Refused::NotTrusted)
        );
        assert_eq!(
            verify(text.as_bytes(), &[[9; 32]], 0),
            Err(Refused::NotTrusted)
        );
        assert_eq!(
            verify(text.as_bytes(), &[pk], 4),
            Err(Refused::Older {
                sequence: 3,
                seen: 4
            })
        );
        let unsigned = text.split(SIGNATURE).next().unwrap();
        assert_eq!(
            verify(unsigned.as_bytes(), &[pk], 0),
            Err(Refused::Unsigned)
        );
        // A second index appended after the signature is not read.
        let tail = format!("{text}[[release]]\n");
        assert!(verify(tail.as_bytes(), &[pk], 0).is_err());
    }

    #[test]
    fn a_host_is_offered_the_newest_release_that_updates_from_it() {
        let v = |s: &str| s.parse::<Release>().unwrap();
        let mut i = index();
        assert_eq!(newest_for(&i, v("V1R0M0")).unwrap().release, "V1R0M1");
        assert!(newest_for(&i, v("V1R0M1")).is_none());
        i = with(i, entry("V1R1M0", "V1R0M1"), 1_793_000_200);
        assert_eq!(i.sequence, 4);
        // From V1R0M0 only V1R0M1 is offered; V1R1M0 wants V1R0M1 first.
        assert_eq!(newest_for(&i, v("V1R0M0")).unwrap().release, "V1R0M1");
        assert_eq!(newest_for(&i, v("V1R0M1")).unwrap().release, "V1R1M0");
        // Published again: replaced, not doubled.
        let again = with(i, entry("V1R1M0", "V1R0M1"), 1_793_000_300);
        assert_eq!(again.releases.len(), 3);
    }
}
