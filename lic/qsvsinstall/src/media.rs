// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The installation media: found among the block devices by its ISO
//! volume identifier, as a virtual or real CD or as the same image on
//! a USB stick, mounted read-only, and verified. The manifest must be
//! signed by a key of this initramfs and every file must have the
//! manifest's length and hash; media that fails either is refused
//! before anything is written.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hostdisk::payload::{self, Manifest};
use sha2::{Digest, Sha256};

/// The trusted signers' public keys, 32 bytes each.
pub const KEYS: &str = "/qsvs/keys/system.pub";
/// Where the media is mounted.
pub const MOUNT: &str = "/run/media";
/// How long the media's device may take to appear.
const WAIT: Duration = Duration::from_secs(20);

/// Where an ISO 9660 primary volume descriptor is, and its fields.
const DESCRIPTOR: u64 = 16 * 2048;
const VOLUME_ID: std::ops::Range<usize> = 40..72;

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

/// The ISO volume identifier of `device`, when it holds an ISO 9660
/// file system.
fn volume_id(device: &Path) -> Option<String> {
    let mut sector = [0u8; 2048];
    let mut file = File::open(device).ok()?;
    file.seek(SeekFrom::Start(DESCRIPTOR)).ok()?;
    file.read_exact(&mut sector).ok()?;
    (sector[0] == 1 && &sector[1..6] == b"CD001").then(|| {
        String::from_utf8_lossy(&sector[VOLUME_ID])
            .trim_end()
            .to_string()
    })
}

/// The block device that holds this release's media; the whole device
/// before any partition of it.
pub fn find() -> Option<PathBuf> {
    let wanted = payload::volume_id(hostdisk::layout::VENDOR, &release::RELEASE.to_string());
    let mut names: Vec<String> = fs::read_dir("/sys/class/block")
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !["loop", "ram", "zram"].iter().any(|p| n.starts_with(p)))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| Path::new("/dev").join(n))
        .find(|device| volume_id(device).as_deref() == Some(wanted.as_str()))
}

/// The media's device, waited for.
pub fn wait() -> Result<PathBuf, String> {
    let give_up = Instant::now() + WAIT;
    loop {
        if let Some(device) = find() {
            return Ok(device);
        }
        if Instant::now() >= give_up {
            return Err("no device holds this release's installation media".into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Mount the media's file system read-only at [`MOUNT`].
#[cfg(target_os = "linux")]
pub fn mount(device: &Path) -> Result<(), String> {
    use rustix::mount::{MountFlags, mount};
    fs::create_dir_all(MOUNT).map_err(|e| format!("{MOUNT}: {e}"))?;
    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC;
    mount(device, MOUNT, "iso9660", flags, None)
        .map_err(|e| format!("{}: the media was not mounted: {e}", device.display()))
}

/// A file of `dir` by its name, whatever case the file system shows.
fn find_in(dir: &Path, name: &str) -> Result<PathBuf, String> {
    fs::read_dir(dir)
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
pub fn signed(text: &[u8], signature: &[u8], keys: &[u8]) -> bool {
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    keys.as_chunks::<32>().0.iter().any(|key| {
        VerifyingKey::from_bytes(key).is_ok_and(|key| key.verify(text, &signature).is_ok())
    })
}

/// Check `bytes` against the manifest's entry `name`.
pub fn check(manifest: &Manifest, name: &str, bytes: &[u8]) -> Result<(), String> {
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

/// Verify the payload under `root` (the mounted media) against `keys`.
pub fn verify(root: &Path, keys: &[u8]) -> Result<Payload, String> {
    let dir = find_in(root, payload::DIR)?;
    let read = |name: &str| -> Result<Vec<u8>, String> {
        let path = find_in(&dir, name)?;
        fs::read(&path).map_err(|e| format!("{name}: {e}"))
    };
    let text = read(payload::MANIFEST)?;
    let signature = read(payload::SIGNATURE)?;
    let signature = hex::decode(String::from_utf8_lossy(&signature).trim())
        .map_err(|_| "the manifest's signature is not hex".to_string())?;
    if !signed(&text, &signature, keys) {
        return Err("the manifest is not signed by a key this installer trusts".into());
    }
    let manifest = Manifest::parse(&String::from_utf8_lossy(&text))?;
    if manifest.release != release::RELEASE.to_string() {
        return Err(format!(
            "the media is of release {}, this installer of {}",
            manifest.release,
            release::RELEASE
        ));
    }
    let slot_a = read(payload::SLOT_A)?;
    check(&manifest, payload::SLOT_A, &slot_a)?;
    let slot_b = read(payload::SLOT_B)?;
    check(&manifest, payload::SLOT_B, &slot_b)?;
    Ok(Payload {
        manifest,
        slot_a,
        slot_b,
    })
}

/// Find, mount and verify the media: its device and its payload.
#[cfg(target_os = "linux")]
pub fn open() -> Result<(PathBuf, Payload), String> {
    let device = wait()?;
    mount(&device)?;
    let keys = fs::read(KEYS).map_err(|e| format!("{KEYS}: {e}"))?;
    let payload = verify(Path::new(MOUNT), &keys)?;
    Ok((device, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use hostdisk::payload::File as Entry;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qsvsinstall-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("qsvs")).unwrap();
        dir
    }

    /// Media as the kernel shows a plain ISO 9660: lower-case names.
    fn media(dir: &Path, key: &SigningKey) {
        let (a, b) = (b"slot a uki".to_vec(), b"slot b uki".to_vec());
        let entry = |name: &str, bytes: &[u8]| Entry {
            name: name.into(),
            bytes: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
        };
        let manifest = Manifest {
            product: "QS/VS".into(),
            release: release::RELEASE.to_string(),
            build: "test".into(),
            files: vec![entry(payload::SLOT_A, &a), entry(payload::SLOT_B, &b)],
        };
        let text = manifest.render();
        let signature = hex::encode(key.sign(text.as_bytes()).to_bytes());
        let put = |name: &str, bytes: &[u8]| {
            fs::write(dir.join("qsvs").join(name.to_lowercase()), bytes).unwrap();
        };
        put(payload::MANIFEST, text.as_bytes());
        put(payload::SIGNATURE, signature.as_bytes());
        put(payload::SLOT_A, &a);
        put(payload::SLOT_B, &b);
    }

    #[test]
    fn media_signed_by_a_trusted_key_verifies() {
        let dir = scratch("good");
        let key = SigningKey::from_bytes(&[7; 32]);
        media(&dir, &key);
        // The trusted keys: another one, then the signer's.
        let other = SigningKey::from_bytes(&[9; 32]);
        let mut keys = other.verifying_key().to_bytes().to_vec();
        keys.extend_from_slice(&key.verifying_key().to_bytes());
        let payload = verify(&dir, &keys).unwrap();
        assert_eq!(payload.slot_a, b"slot a uki");
        assert_eq!(payload.manifest.files.len(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn media_is_refused_for_its_signature_its_files_or_its_release() {
        let dir = scratch("bad");
        let key = SigningKey::from_bytes(&[7; 32]);
        let keys = key.verifying_key().to_bytes();
        let file = |name: &str| dir.join("qsvs").join(name.to_lowercase());

        // Signed by a key that is not trusted.
        media(&dir, &SigningKey::from_bytes(&[8; 32]));
        assert!(verify(&dir, &keys).unwrap_err().contains("not signed"));

        // The manifest changed after it was signed.
        media(&dir, &key);
        let text = fs::read_to_string(file(payload::MANIFEST)).unwrap();
        fs::write(
            file(payload::MANIFEST),
            text.replace("build test", "build evil"),
        )
        .unwrap();
        assert!(verify(&dir, &keys).unwrap_err().contains("not signed"));

        // A UKI changed: one byte, then its length.
        media(&dir, &key);
        fs::write(file(payload::SLOT_B), b"slot B uki").unwrap();
        assert!(verify(&dir, &keys).unwrap_err().contains("manifest's hash"));
        fs::write(file(payload::SLOT_B), b"slot b uki and more").unwrap();
        assert!(verify(&dir, &keys).unwrap_err().contains("bytes"));

        // A file missing; no signature at all.
        media(&dir, &key);
        fs::remove_file(file(payload::SLOT_A)).unwrap();
        assert!(verify(&dir, &keys).unwrap_err().contains("no SLOTA.EFI"));
        media(&dir, &key);
        fs::write(file(payload::SIGNATURE), "zz").unwrap();
        assert!(verify(&dir, &keys).unwrap_err().contains("not hex"));
        assert!(!signed(b"text", &[0; 64], &keys));
        assert!(!signed(b"text", &[0; 10], &keys));
        fs::remove_dir_all(&dir).unwrap();
    }
}
