// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the installation media carries beside its own boot image: the
//! two slots' UKIs and a manifest of them, signed by the build's key.
//! The build writes it; the installer finds the media by its volume
//! identifier, checks the signature against the keys in its own
//! initramfs and every file against the manifest ([`verify`]), before
//! it writes anything.
//!
//! The manifest is text, a word and its values a line:
//!
//! The first word is the appliance's vendor name, which is also the
//! payload's directory on the media.
//!
//! ```text
//! QSDEMO-PAYLOAD 1
//! product QS/DEMO
//! release V1R0M0
//! build a1b2c3d
//! file SLOTA.EFI 12837888 7824c4c3...
//! file SLOTB.EFI 12837888 531a6a4f...
//! ```
//!
//! This is not the artifact manifest of a build (`manifest.json`):
//! that one lists the ISO, so it cannot be on it.

use std::fmt::Write as _;

use appliance::LoadSourceNames;

/// Slot A's UKI in the payload's directory.
pub const SLOT_A: &str = "SLOTA.EFI";
/// Slot B's UKI there.
pub const SLOT_B: &str = "SLOTB.EFI";
/// The manifest there.
pub const MANIFEST: &str = "PAYLOAD.TXT";
/// The manifest's signature there: ed25519, 128 hex digits.
pub const SIGNATURE: &str = "PAYLOAD.SIG";

/// The directory of the payload on the media.
pub fn dir(names: &LoadSourceNames) -> &'static str {
    names.vendor
}

/// The manifest's first line.
pub fn head(names: &LoadSourceNames) -> String {
    format!("{}-PAYLOAD 1", names.vendor)
}

/// The ISO volume identifier of a product's media at a release.
pub fn volume_id(vendor: &str, release: &str) -> String {
    format!("{vendor}_{release}")
}

/// One file of the payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Its name in the payload's directory.
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
    pub fn render(&self, names: &LoadSourceNames) -> String {
        let mut out = format!(
            "{}\nproduct {}\nrelease {}\nbuild {}\n",
            head(names),
            self.product,
            self.release,
            self.build
        );
        for file in &self.files {
            let _ = writeln!(out, "file {} {} {}", file.name, file.bytes, file.sha256);
        }
        out
    }

    /// The manifest of `text`, or why it is not one.
    pub fn parse(names: &LoadSourceNames, text: &str) -> Result<Self, String> {
        let head = head(names);
        let mut lines = text.lines();
        if lines.next() != Some(head.as_str()) {
            return Err(format!("the manifest does not start with `{head}`"));
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

/// The payload, verified and in memory.
#[derive(Debug)]
pub struct Payload {
    /// The manifest.
    pub manifest: Manifest,
    /// Slot A's UKI.
    pub slot_a: Vec<u8>,
    /// Slot B's UKI.
    pub slot_b: Vec<u8>,
}

/// Where an ISO 9660 primary volume descriptor is, and its fields.
const DESCRIPTOR: u64 = 16 * 2048;
const VOLUME_ID: std::ops::Range<usize> = 40..72;

/// The ISO volume identifier of `device`, when it holds an ISO 9660
/// file system.
pub fn device_volume_id(device: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut sector = [0u8; 2048];
    let mut file = std::fs::File::open(device).ok()?;
    file.seek(SeekFrom::Start(DESCRIPTOR)).ok()?;
    file.read_exact(&mut sector).ok()?;
    (sector[0] == 1 && &sector[1..6] == b"CD001").then(|| {
        String::from_utf8_lossy(&sector[VOLUME_ID])
            .trim_end()
            .to_string()
    })
}

/// The block device that holds the media of `release` of the appliance
/// `names` names, by its volume identifier: the whole device before
/// any partition of it, a virtual or real CD or the same image on a USB
/// stick.
pub fn find_media(names: &LoadSourceNames, release: &str) -> Option<std::path::PathBuf> {
    let wanted = volume_id(names.vendor, release);
    let mut found: Vec<String> = std::fs::read_dir("/sys/class/block")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !["loop", "ram", "zram"].iter().any(|p| n.starts_with(p)))
        .collect();
    found.sort();
    found
        .into_iter()
        .map(|n| std::path::Path::new("/dev").join(n))
        .find(|device| device_volume_id(device).as_deref() == Some(wanted.as_str()))
}

/// [`find_media`], waited for `within`.
///
/// # Errors
/// No device held it in time.
pub fn wait_for_media(
    names: &LoadSourceNames,
    release: &str,
    within: std::time::Duration,
) -> Result<std::path::PathBuf, String> {
    let give_up = std::time::Instant::now() + within;
    loop {
        if let Some(device) = find_media(names, release) {
            return Ok(device);
        }
        if std::time::Instant::now() >= give_up {
            return Err("no device holds this release's installation media".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Mount the media's file system read-only at `at`.
///
/// # Errors
/// It was not mounted.
#[cfg(target_os = "linux")]
pub fn mount_media(device: &std::path::Path, at: &str) -> Result<(), String> {
    use rustix::mount::{MountFlags, mount};
    std::fs::create_dir_all(at).map_err(|e| format!("{at}: {e}"))?;
    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC;
    mount(device, at, "iso9660", flags, None)
        .map_err(|e| format!("{}: the media was not mounted: {e}", device.display()))
}

/// A file of `dir` by its name, whatever case the file system shows.
fn find_in(dir: &std::path::Path, name: &str) -> Result<std::path::PathBuf, String> {
    std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .find(|e| {
            let found = e.file_name().to_string_lossy().into_owned();
            // A plain ISO 9660 name may carry its version.
            found
                .split(';')
                .next()
                .unwrap_or("")
                .eq_ignore_ascii_case(name)
        })
        .map(|e| e.path())
        .ok_or_else(|| format!("the media has no {name}"))
}

/// Whether `signature` over `text` is by one of `keys` (32 bytes each).
#[must_use]
pub fn signed(text: &[u8], signature: &[u8], keys: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    keys.as_chunks::<32>().0.iter().any(|key| {
        VerifyingKey::from_bytes(key).is_ok_and(|key| key.verify(text, &signature).is_ok())
    })
}

/// Check `bytes` against the manifest's entry `name`.
///
/// # Errors
/// Not listed, or not of its length or hash.
pub fn matches(manifest: &Manifest, name: &str, bytes: &[u8]) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let entry = manifest
        .file(name)
        .ok_or_else(|| format!("the manifest does not list {name}"))?;
    if entry.bytes != bytes.len() as u64 {
        return Err(format!(
            "{name} is {} bytes; the manifest says {}",
            bytes.len(),
            entry.bytes
        ));
    }
    let hash = hex::encode(Sha256::digest(bytes));
    if hash != entry.sha256 {
        return Err(format!("{name} does not have the manifest's hash"));
    }
    Ok(())
}

/// Verify the payload of the appliance `names` names under `root` (the
/// mounted media) against `keys` (32 bytes each): the manifest signed
/// by one of them, of `release`, and every slot's file its length and
/// hash. Nothing is written before this has answered.
///
/// # Errors
/// What was found wrong.
pub fn verify(
    root: &std::path::Path,
    names: &LoadSourceNames,
    release: &str,
    keys: &[u8],
) -> Result<Payload, String> {
    let dir = find_in(root, dir(names))?;
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = find_in(&dir, name)?;
        std::fs::read(&path).map_err(|e| format!("{name}: {e}"))
    };
    let text = read(MANIFEST)?;
    let signature = read(SIGNATURE)?;
    let signature = hex::decode(String::from_utf8_lossy(&signature).trim())
        .map_err(|_| "the manifest's signature is not hex".to_string())?;
    if !signed(&text, &signature, keys) {
        return Err("the manifest is not signed by a key this installer trusts".into());
    }
    let manifest = Manifest::parse(names, &String::from_utf8_lossy(&text))?;
    if manifest.release != release {
        return Err(format!(
            "the media is of release {}, this installer of {release}",
            manifest.release
        ));
    }
    let slot_a = read(SLOT_A)?;
    matches(&manifest, SLOT_A, &slot_a)?;
    let slot_b = read(SLOT_B)?;
    matches(&manifest, SLOT_B, &slot_b)?;
    Ok(Payload {
        manifest,
        slot_a,
        slot_b,
    })
}

/// Find, mount at `at` and verify the media of `release`, with the keys
/// in the file `keys`: its device and its payload.
///
/// # Errors
/// As each step.
#[cfg(target_os = "linux")]
pub fn open(
    names: &LoadSourceNames,
    release: &str,
    keys: &std::path::Path,
    at: &str,
    within: std::time::Duration,
) -> Result<(std::path::PathBuf, Payload), String> {
    let device = wait_for_media(names, release, within)?;
    mount_media(&device, at)?;
    let keys = std::fs::read(keys).map_err(|e| format!("{}: {e}", keys.display()))?;
    let payload = verify(std::path::Path::new(at), names, release, &keys)?;
    Ok((device, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &LoadSourceNames = &LoadSourceNames::DEMO;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("payload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(dir_name())).unwrap();
        dir
    }

    fn dir_name() -> String {
        super::dir(NAMES).to_lowercase()
    }

    /// Media as the kernel shows a plain ISO 9660: lower-case names.
    fn media(at: &std::path::Path, key: &ed25519_dalek::SigningKey) {
        use ed25519_dalek::Signer;
        use sha2::{Digest, Sha256};
        let (a, b) = (b"slot a uki".to_vec(), b"slot b uki".to_vec());
        let entry = |name: &str, bytes: &[u8]| File {
            name: name.into(),
            bytes: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        };
        let manifest = Manifest {
            product: "QS/DEMO".into(),
            release: "V1R0M0".into(),
            build: "test".into(),
            files: vec![entry(SLOT_A, &a), entry(SLOT_B, &b)],
        };
        let text = manifest.render(NAMES);
        let signature = hex::encode(key.sign(text.as_bytes()).to_bytes());
        let put = |name: &str, bytes: &[u8]| {
            std::fs::write(at.join(dir_name()).join(name.to_lowercase()), bytes).unwrap();
        };
        put(MANIFEST, text.as_bytes());
        put(SIGNATURE, signature.as_bytes());
        put(SLOT_A, &a);
        put(SLOT_B, &b);
    }

    #[test]
    fn media_signed_by_a_trusted_key_verifies() {
        use ed25519_dalek::SigningKey;
        let dir = scratch("good");
        let key = SigningKey::from_bytes(&[7; 32]);
        media(&dir, &key);
        // The trusted keys: another one, then the signer's.
        let other = SigningKey::from_bytes(&[9; 32]);
        let mut keys = other.verifying_key().to_bytes().to_vec();
        keys.extend_from_slice(&key.verifying_key().to_bytes());
        let payload = verify(&dir, NAMES, "V1R0M0", &keys).unwrap();
        assert_eq!(payload.slot_a, b"slot a uki");
        assert_eq!(payload.manifest.files.len(), 2);
        // Another release's installer refuses it.
        let other = verify(&dir, NAMES, "V1R0M1", &keys).unwrap_err();
        assert!(other.contains("release V1R0M0"), "{other}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn media_is_refused_for_its_signature_or_its_files() {
        use ed25519_dalek::SigningKey;
        let dir = scratch("bad");
        let key = SigningKey::from_bytes(&[7; 32]);
        let keys = key.verifying_key().to_bytes();
        let file = |name: &str| dir.join(dir_name()).join(name.to_lowercase());
        let v = |dir: &std::path::Path| verify(dir, NAMES, "V1R0M0", &keys);

        // Signed by a key that is not trusted.
        media(&dir, &SigningKey::from_bytes(&[8; 32]));
        assert!(v(&dir).unwrap_err().contains("not signed"));

        // The manifest changed after it was signed.
        media(&dir, &key);
        let text = std::fs::read_to_string(file(MANIFEST)).unwrap();
        std::fs::write(file(MANIFEST), text.replace("build test", "build evil")).unwrap();
        assert!(v(&dir).unwrap_err().contains("not signed"));

        // A UKI changed: one byte, then its length.
        media(&dir, &key);
        std::fs::write(file(SLOT_B), b"slot B uki").unwrap();
        assert!(v(&dir).unwrap_err().contains("manifest's hash"));
        std::fs::write(file(SLOT_B), b"slot b uki and more").unwrap();
        assert!(v(&dir).unwrap_err().contains("bytes"));

        // A file missing; no signature at all.
        media(&dir, &key);
        std::fs::remove_file(file(SLOT_A)).unwrap();
        assert!(v(&dir).unwrap_err().contains("no SLOTA.EFI"));
        media(&dir, &key);
        std::fs::write(file(SIGNATURE), "zz").unwrap();
        assert!(v(&dir).unwrap_err().contains("not hex"));
        assert!(!signed(b"text", &[0; 64], &keys));
        assert!(!signed(b"text", &[0; 10], &keys));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn manifest() -> Manifest {
        Manifest {
            product: "QS/DEMO (test image)".into(),
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
        let text = m.render(NAMES);
        assert!(text.starts_with("QSDEMO-PAYLOAD 1\nproduct QS/DEMO (test image)\n"));
        assert_eq!(Manifest::parse(NAMES, &text), Ok(m.clone()));
        assert_eq!(m.file(SLOT_B).unwrap().sha256.len(), 64);
        assert!(m.file("OTHER").is_none());
        assert_eq!(volume_id(dir(NAMES), "V1R0M0"), "QSDEMO_V1R0M0");
    }

    #[test]
    fn what_is_not_a_manifest_is_refused() {
        let text = manifest().render(NAMES);
        for bad in [
            text.replacen("QSDEMO-PAYLOAD 1", "QSDEMO-PAYLOAD 2", 1),
            text.replacen("file SLOTA.EFI 12837888", "file SLOTA.EFI many", 1),
            format!("{text}extra line\n"),
            text.replacen("product QS/DEMO (test image)\n", "", 1),
            text.replacen(&"7824c4c3".repeat(8), "abc", 1),
        ] {
            assert!(Manifest::parse(NAMES, &bad).is_err(), "{bad}");
        }
    }
}
