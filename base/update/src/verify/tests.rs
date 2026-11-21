// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use ed25519_dalek::{Signer, SigningKey};

use super::*;
use crate::manifest::File;

const V1R0M0: Release = Release { v: 1, r: 0, m: 0 };

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

/// An image of `release` for `product`, signed by `signer`.
fn image(product: &str, release: &str, signer: &SigningKey) -> Vec<u8> {
    let (a, b) = (vec![0xA5u8; 3000], vec![0x5Au8; 3000]);
    let m = Manifest {
        product: product.into(),
        release: release.into(),
        build: "test".into(),
        fixes: vec!["DM00001".into()],
        schemas: vec![("host.toml".into(), 1), ("definition.toml".into(), 2)],
        files: vec![
            File {
                name: SLOT_A.into(),
                bytes: a.len() as u64,
                sha256: hex::encode(Sha256::digest(&a)),
            },
            File {
                name: SLOT_B.into(),
                bytes: b.len() as u64,
                sha256: hex::encode(Sha256::digest(&b)),
            },
        ],
    };
    crate::make(&crate::DEMO, &m, &a, &b, |text| {
        signer.sign(text).to_bytes()
    })
}

fn running() -> Running {
    Running {
        format: &crate::DEMO,
        product: "DEMO".into(),
        release: V1R0M0,
        keys: [
            key(9).verifying_key().to_bytes(),
            key(1).verifying_key().to_bytes(),
        ]
        .concat(),
        state: vec![("host.toml".into(), 1), ("definition.toml".into(), 2)],
    }
}

#[test]
fn a_signed_image_of_a_newer_release_verifies() {
    let img = image("DEMO", "V1R0M1", &key(1));
    let v = verify(&img, &running(), false).unwrap();
    assert_eq!(v.manifest.release, "V1R0M1");
    assert_eq!(&img[v.slots[1].clone()][..2], &[0x5A, 0x5A]);
    // The same release again verifies too: a rebuild.
    assert!(verify(&image("DEMO", "V1R0M0", &key(1)), &running(), false).is_ok());
}

#[test]
fn each_refusal_says_which_check() {
    let r = running();
    assert_eq!(
        verify(&image("DEMO", "V1R0M1", &key(2)), &r, false),
        Err(Refusal::Signature)
    );
    let mut damaged = image("DEMO", "V1R0M1", &key(1));
    let last = damaged.len() - 1;
    damaged[last] ^= 1;
    assert!(matches!(verify(&damaged, &r, false), Err(Refusal::Damaged(w)) if w.contains("hash")));
    assert!(matches!(
        verify(&damaged[..100], &r, false),
        Err(Refusal::Damaged(_))
    ));
    assert!(matches!(
        verify(&image("OTHER", "V1R0M1", &key(1)), &r, false),
        Err(Refusal::Product { .. })
    ));
    let older = Running {
        release: Release { v: 1, r: 0, m: 1 },
        ..running()
    };
    let img = image("DEMO", "V1R0M0", &key(1));
    assert!(matches!(
        verify(&img, &older, false),
        Err(Refusal::Older { .. })
    ));
    assert!(verify(&img, &older, true).is_ok(), "older with leave");
    // State the image's release cannot read: refused even with leave.
    let newer_state = Running {
        state: vec![("definition.toml".into(), 3)],
        ..older
    };
    assert_eq!(
        verify(&img, &newer_state, true),
        Err(Refusal::State {
            file: "definition.toml".into(),
            found: 3,
            reads: 2
        })
    );
    // A signature over another manifest: the manifest changed after it
    // was signed.
    let mut changed = image("DEMO", "V1R0M1", &key(1));
    let at = changed.windows(6).position(|w| w == b"V1R0M1").unwrap();
    changed[at + 5] = b'9';
    assert_eq!(verify(&changed, &r, false), Err(Refusal::Signature));
}

#[test]
fn the_state_as_it_stands_is_read_with_its_versions() {
    let d = std::env::temp_dir().join(format!("update-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("partitions/1")).unwrap();
    std::fs::create_dir_all(d.join("partitions/2")).unwrap();
    std::fs::write(d.join("host.toml"), "schema_version = 1\nname = \"X\"\n").unwrap();
    std::fs::write(
        d.join("partitions/1/definition.toml"),
        "schema_version = 2\n",
    )
    .unwrap();
    std::fs::write(
        d.join("partitions/2/definition.toml"),
        "schema_version = 3\n",
    )
    .unwrap();
    std::fs::write(d.join("users.db"), "not toml").unwrap();
    assert_eq!(
        state_versions(&d),
        vec![("definition.toml".into(), 3), ("host.toml".into(), 1)]
    );
    std::fs::remove_dir_all(&d).unwrap();
}
