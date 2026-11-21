// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Code update of an appliance whose code is in two slots: an update
//! image staged on its data file system, verified against the running
//! image's trusted keys, installed into the slot that is not running on
//! every load-source disk, marked pending, and activated by a restart
//! into it; a slot that does not confirm rolls back. Around the pending
//! slot's first IPL the state is copied, and put back when it rolls
//! back ([`state_copy`]).
//!
//! The product gives what is its own: the image's [`Format`] (its
//! magic, its manifest's first line, its fixes' prefix), its name and
//! release, the state schemas it reads, the [`Messages`] it refuses
//! with, the [`Code`]s it posts, and the [`Partitions`] (or whatever it
//! runs) that must be stopped before a restart.
//!
//! - [`manifest`]: what an image is, as signed text.
//! - [`image`]: the image's container, read as hostile.
//! - [`verify`]: the checks an image passes before it is installed.
//! - [`proto`]: the update service's socket.
//! - [`state_copy`]: the state kept across a pending slot's trial.
//! - `serve` (Linux): the service: staging, the install, the slots.

#![forbid(unsafe_code)]

pub mod image;
pub mod manifest;
pub mod proto;
#[cfg(target_os = "linux")]
pub mod serve;
pub mod state_copy;
pub mod verify;

pub use manifest::Manifest;
pub use verify::{Refusal, Running, Verified};

/// What an update image of the product is on the wire: the product's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// The container's first eight bytes.
    pub magic: [u8; 8],
    /// The manifest's first line.
    pub head: &'static str,
    /// The two letters a fix's name starts with, five digits after.
    pub fix_prefix: &'static str,
}

/// An appliance of no product's, for tests and the fuzzer's seeds.
pub const DEMO: Format = Format {
    magic: *b"DEMOUPDT",
    head: "DEMO-UPDATE 1",
    fix_prefix: "DM",
};

/// The messages the service refuses with, the product's.
#[derive(Clone, Copy, Debug)]
#[allow(missing_docs)] // each is named for its refusal
pub struct Messages {
    pub busy: &'static msg::Message,
    pub cut: &'static msg::Message,
    pub damaged: &'static msg::Message,
    pub name: &'static msg::Message,
    pub not_kept: &'static msg::Message,
    pub not_verified: &'static msg::Message,
    pub not_written: &'static msg::Message,
    pub nothing_pending: &'static msg::Message,
    pub nothing_staged: &'static msg::Message,
    pub older: &'static msg::Message,
    pub partitions_run: &'static msg::Message,
    pub pending: &'static msg::Message,
    pub product: &'static msg::Message,
    pub record: &'static msg::Message,
    pub signature: &'static msg::Message,
    pub staged_already: &'static msg::Message,
    pub state: &'static msg::Message,
}

/// What the service posts, each as the product's reference code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// An image was staged whole.
    Staged,
    /// It verified.
    Verified,
    /// It was refused.
    Refused,
    /// What was staged was thrown away.
    Discarded,
    /// An install began.
    Installing,
    /// A slot was written on one disk.
    SlotWritten,
    /// The install ended: the slot is pending.
    Installed,
    /// The install failed.
    InstallFailed,
    /// The pending slot is pending no more.
    Cancelled,
}

/// What must be stopped before the restart into a pending slot: a
/// host's partitions, nothing on an appliance that runs none.
pub trait Partitions {
    /// What runs now: each one's ID and name.
    fn running(&mut self) -> Vec<(u32, String)>;
    /// Shut each of `ids` down through its own system, each given
    /// `timeout` seconds, and end at once what is still active after.
    fn end(&mut self, ids: &[u32], timeout: u32);
}

/// An appliance that runs nothing to stop.
#[derive(Clone, Copy, Debug, Default)]
pub struct Nothing;

impl Partitions for Nothing {
    fn running(&mut self) -> Vec<(u32, String)> {
        Vec::new()
    }
    fn end(&mut self, _: &[u32], _: u32) {}
}

/// A slot, as a posting says what it is about.
#[must_use]
pub fn slot_about(letter: &str) -> supervise::control::About {
    supervise::control::About {
        kind: supervise::control::about::CODE_SLOT.into(),
        name: letter.into(),
        ..supervise::control::About::default()
    }
}

/// The image of `manifest` holding `slot_a` and `slot_b` in `format`,
/// its manifest signed by `sign`.
pub fn make(
    format: &Format,
    manifest: &Manifest,
    slot_a: &[u8],
    slot_b: &[u8],
    sign: impl Fn(&[u8]) -> [u8; image::SIGNATURE],
) -> Vec<u8> {
    let text = manifest.render(format);
    let signature = sign(text.as_bytes());
    image::write(
        format,
        &text,
        &signature,
        &[(verify::SLOT_A, slot_a), (verify::SLOT_B, slot_b)],
    )
}

/// One input of the fuzzer: the image's parser, its manifest's and the
/// verification, each read as hostile, in `format`.
pub fn fuzz_image(format: &'static Format, data: &[u8]) {
    if let Ok(p) = image::parse(format, data) {
        let _ = std::str::from_utf8(&data[p.manifest.clone()]).map(|t| Manifest::parse(format, t));
    }
    let _ = verify::verify(
        data,
        &Running {
            format,
            product: "DEMO".into(),
            release: vrm::Release { v: 1, r: 0, m: 0 },
            keys: vec![0x42; 32],
            state: vec![("host.toml".into(), 1)],
        },
        false,
    );
}

/// One input of the fuzzer: a manifest's text alone, in `format`.
pub fn fuzz_manifest(format: &Format, data: &[u8]) {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(m) = Manifest::parse(format, text)
    {
        // What reads renders to what reads the same.
        assert_eq!(Manifest::parse(format, &m.render(format)).as_ref(), Ok(&m));
    }
}
