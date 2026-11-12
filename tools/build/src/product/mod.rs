// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS as `buildkit` sees it: the names, the kernel's needs, the
//! userland, the initramfs, the command lines, the disk and the media.
//!
//! There are two kinds of image. The release image is what ships. The
//! test image is the same with the test hooks added (`qsvshook`, and
//! PID 1 built under `cfg(test_hooks)`); it is built under its own
//! directory of the target root and its manifest names it a test image.

mod initramfs;
pub mod kernel;
mod media;

use std::path::Path;

use anyhow::{Context, Result};
use buildkit::{Notices, Package, Product, UkiKind};
use hostdisk::layout;

/// The PID 1 package, `/init` in every initramfs.
pub const INIT: &str = "qsvsinit";

/// The service packages, `/qsvs/bin/<name>` in every initramfs.
pub const SERVICES: [&str; 4] = ["qsvsstgd", "qsvslogd", "qsvsnetd", "qsvstools"];

/// The installer's package, `/qsvs/bin/qsvsinstall` in the media
/// UKI's initramfs only.
pub const INSTALLER: &str = "qsvsinstall";

/// The test hooks' package: in a test image only.
pub const HOOK: &str = "qsvshook";

/// The environment variable that builds PID 1 with its test hooks.
pub const HOOKS_ENV: &str = "QSVS_TEST_HOOKS";

/// What PID 1 of a test image prints, and no other PID 1 contains.
pub const HOOKS_MARKER: &str = "TEST IMAGE:";

/// The test image's directory under the target root.
pub const TEST_DIR: &str = "test-image";

/// The manifest's `product` for a test image.
pub const TEST_NAME: &str = "QS/VS (test image)";

/// Which image is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Image {
    /// What ships: no test hooks.
    Release,
    /// The release image with the test hooks added. Never shipped.
    Test,
}

const fn package(name: &'static str) -> Package {
    Package {
        name,
        manifest: true,
    }
}

const RELEASE_PACKAGES: &[Package] = &[
    package(INIT),
    package(SERVICES[0]),
    package(SERVICES[1]),
    package(SERVICES[2]),
    package(SERVICES[3]),
    package(INSTALLER),
];
const TEST_PACKAGES: &[Package] = &[
    package(INIT),
    package(SERVICES[0]),
    package(SERVICES[1]),
    package(SERVICES[2]),
    package(SERVICES[3]),
    package(INSTALLER),
    package(HOOK),
];

/// Where the generated notices file lives in the tree.
pub const NOTICES_FILE: &str = "lic/release/NOTICES.txt";

/// QS/VS, from this crate's place in the repository.
pub fn qsvs(image: Image) -> Result<Product> {
    let (name, packages, initramfs): (_, _, fn(&_, _) -> _) = match image {
        Image::Release => (release::NAME, RELEASE_PACKAGES, initramfs::entries),
        Image::Test => (TEST_NAME, TEST_PACKAGES, initramfs::test_entries),
    };
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .context("build crate is not under tools/")?
        .to_path_buf();
    Ok(Product {
        id: "qsvs",
        name,
        vendor: layout::VENDOR,
        env_prefix: release::ENV_PREFIX,
        key_prefix: release::KEY_PREFIX,
        repo,
        release: release::RELEASE.to_string(),
        build_id: release::BUILD_ID.to_string(),
        fragments: &[kernel::FRAGMENT],
        required_y: kernel::REQUIRED_Y,
        required_n: kernel::REQUIRED_N,
        cmdline: kernel::CMDLINE,
        packages,
        init: INIT,
        kinds: &UkiKind::ALL,
        uki_cmdline,
        initramfs,
        disk_bytes: layout::DEFAULT_DISK_BYTES,
        disk_parts: layout::parts,
        disk_guids: layout::fixed_guids,
        esp_label: layout::ESP_LABEL,
        esp_volume_id: layout::ESP_VOLUME_ID,
        media: Some(media::media),
        doctor: crate::host::doctor::rows,
        deny: &[],
        notices: Some(Notices {
            file: NOTICES_FILE,
            intro: notices_intro,
        }),
        secure_boot_db: || None,
    })
}

/// The slots differ only in `qsvs.slot=`; the media UKI carries
/// `qsvs.install=1` instead of a slot. There are no development words.
fn uki_cmdline(p: &Product, kind: UkiKind, _dev: bool) -> String {
    let base = p.cmdline;
    match kind {
        UkiKind::SlotA => format!("{base} {}=A", p.key("slot")),
        UkiKind::SlotB => format!("{base} {}=B", p.key("slot")),
        UkiKind::Media => format!("{base} {}=1", p.key("install")),
    }
}

/// The opening lines of `NOTICES.txt`.
fn notices_intro(release: &str, kernel_version: &str, crates: usize) -> String {
    format!(
        "QS/VS {release} third-party notices\n\
         \nQS/VS is licensed under the MIT license (the LICENSE file of its source).\n\
         It contains the work of others, listed here with their licenses:\n\n\
         1. The Linux kernel {kernel_version}, under GPL-2.0 with the Linux syscall note.\n\
         \x20  No firmware is built into the kernel (CONFIG_EXTRA_FIRMWARE is empty).\n\
         2. musl libc, under MIT: Copyright (c) 2005-2020 Rich Felker, et al.\n\
         \x20  musl's COPYRIGHT file names its authors and the few files it holds\n\
         \x20  under other permissive terms.\n\
         3. The Rust standard library (core, alloc, std and their dependencies),\n\
         \x20  under MIT OR Apache-2.0, taken under MIT; LLVM's libunwind and\n\
         \x20  compiler-rt builtins, under Apache-2.0 WITH LLVM-exception.\n\
         4. The {crates} Rust crates below, each under the license named. The\n\
         \x20  crates of QS/400 (Quartz Systems, MIT) are among them: QS/VS takes\n\
         \x20  its build pipeline and disk formats from QS/400 as libraries.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_lines_are_the_baked_ones() {
        let p = qsvs(Image::Release).unwrap();
        let base = "console=ttyS0,115200 console=tty0 panic=-1 quiet";
        let line = |kind| (p.uki_cmdline)(&p, kind, true);
        assert_eq!(line(UkiKind::SlotA), format!("{base} qsvs.slot=A"));
        assert_eq!(line(UkiKind::SlotB), format!("{base} qsvs.slot=B"));
        assert_eq!(line(UkiKind::Media), format!("{base} qsvs.install=1"));
        // Nothing is added for a development build.
        assert_eq!(
            (p.uki_cmdline)(&p, UkiKind::SlotA, false),
            line(UkiKind::SlotA)
        );
    }

    #[test]
    fn the_names_are_qsvs_own() {
        let p = qsvs(Image::Release).unwrap();
        assert_eq!((p.id, p.vendor), ("qsvs", "QSVS"));
        assert_eq!((p.env_prefix, p.key_prefix), ("QSVS_", "qsvs."));
        assert_eq!(buildkit::image::disk::slot_b(&p), layout::SLOT_B_PATH);
        assert_eq!(buildkit::image::disk::SLOT_A, layout::SLOT_A_PATH);
        let spec = p.fat_spec(diskimg::fat::DISK_ESP).unwrap();
        assert_eq!(&spec.label, b"QSVS ESP   ");
        assert_eq!(spec.volume_id, 0x5153_5653);
        assert!(p.repo.join("VERSION").is_file());
    }

    #[test]
    fn only_a_test_image_has_the_hooks_package() {
        let names = |image| -> Vec<&str> {
            let p = qsvs(image).unwrap();
            p.packages.iter().map(|p| p.name).collect()
        };
        assert_eq!(
            names(Image::Release),
            [
                "qsvsinit",
                "qsvsstgd",
                "qsvslogd",
                "qsvsnetd",
                "qsvstools",
                "qsvsinstall"
            ]
        );
        assert_eq!(
            names(Image::Test),
            [
                "qsvsinit",
                "qsvsstgd",
                "qsvslogd",
                "qsvsnetd",
                "qsvstools",
                "qsvsinstall",
                "qsvshook"
            ]
        );
        assert_eq!(qsvs(Image::Release).unwrap().name, "QS/VS");
        assert_ne!(qsvs(Image::Test).unwrap().name, "QS/VS");
        // Every package is a member of the workspace.
        let repo = qsvs(Image::Test).unwrap().repo;
        let manifest = std::fs::read_to_string(repo.join("Cargo.toml")).unwrap();
        for name in names(Image::Test) {
            // Service Tools is the one that is not Licensed Internal Code.
            let member = if name == SERVICES[3] {
                "svc/tools".to_string()
            } else {
                format!("lic/{name}")
            };
            assert!(manifest.contains(&member), "{name}");
        }
    }
}
