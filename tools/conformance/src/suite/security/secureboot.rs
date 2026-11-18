// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Secure Boot (`docs/RELEASING.md`, `docs/INSTALL.md`). A release's
//! UKIs, signed with the db key by `build release`, start under OVMF's
//! Secure Boot build once the db certificate (`QSVS_SB_CERT`) is
//! enrolled beside the firmware's own keys, and the host says Secure
//! Boot is on; without the certificate the firmware refuses them, and it
//! refuses the test image's, which are not signed.

use std::fs;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::qemu::Launch;
use crate::qemu::image::sparse_copy;
use crate::suite::banner_prefix;
use crate::suite::base::{Run, posted};

/// OVMF's Secure Boot build, and its variables with the firmware
/// makers' keys enrolled and Secure Boot on.
const CODE: &str = "/usr/share/OVMF/OVMF_CODE_4M.secboot.fd";
const VARS: &str = "/usr/share/OVMF/OVMF_VARS_4M.ms.fd";
/// The db certificate a release's UKIs are signed for.
const CERT_ENV: &str = "QSVS_SB_CERT";
/// How long a refused image is given to show its banner anyway.
const REFUSED_WAIT: Duration = Duration::from_secs(90);

/// Slot A's UKI on `disk`'s ESP.
fn slot_a(disk: &Path) -> Result<Vec<u8>, String> {
    let mut f = fs::File::open(disk).map_err(|e| format!("{}: {e}", disk.display()))?;
    let bytes = f.metadata().map_err(|e| e.to_string())?.len();
    let table = ipl::gpt::read(&mut f, bytes).map_err(|e| e.to_string())?;
    let esp = table
        .iter()
        .find(|p| p.type_guid == ipl::ESP_TYPE)
        .ok_or("the disk has no ESP")?;
    let start = esp.first_lba * 512;
    let len = (esp.last_lba + 1 - esp.first_lba) * 512;
    let mut part = vec![0u8; usize::try_from(len).map_err(|e| e.to_string())?];
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    f.read_exact(&mut part).map_err(|e| e.to_string())?;
    let mut volume = ipl::fat::Volume::open(Cursor::new(part), len).map_err(|e| e.to_string())?;
    volume
        .read_file(ipl::SIDE_A_PATH, 256 << 20)
        .map_err(|e| format!("{}: {e}", ipl::SIDE_A_PATH))
}

/// Boot `disk` under the Secure Boot firmware with `vars`.
fn launch(run: &Run<'_>, disk: &Path, vars: &Path) -> Launch {
    Launch {
        disk: Some(disk.to_path_buf()),
        snapshot: true,
        firmware: CODE.into(),
        vars: Some(vars.to_path_buf()),
        secure_boot: true,
        iommu: false,
        ..run.launch()
    }
}

/// `disk` with `vars` never shows its banner: the firmware refused it.
fn refused(run: &Run<'_>, disk: &Path, vars: &Path, what: &str) -> Result<(), String> {
    let mut guest = run.boot(&launch(run, disk, vars))?;
    let shown = guest.seen(&banner_prefix(), Instant::now() + REFUSED_WAIT);
    guest.kill();
    match shown {
        Ok(()) => Err(format!("{what} started under Secure Boot")),
        Err(_) => Ok(()),
    }
}

/// The suite.
pub fn secure_boot(run: &Run<'_>) -> Result<String, String> {
    let cert_path = std::env::var_os(CERT_ENV).ok_or_else(|| {
        format!("{CERT_ENV} is not set: the db certificate `build release` signed with")
    })?;
    let cert = fs::read(&cert_path).map_err(|e| format!("{CERT_ENV}: {e}"))?;
    let release = run
        .release_disk
        .ok_or("no release disk image: `build release` makes one")?;
    pesign::verify(&slot_a(release)?, &cert).map_err(|e| {
        format!("the release disk's UKI is not signed with {CERT_ENV} ({e}): build it with `build release`")
    })?;
    let disk = run.file("sb-release.img");
    sparse_copy(release, &disk)?;
    let ms = fs::read(VARS).map_err(|e| format!("{VARS}: {e}"))?;
    let enrolled = run.file("sb-enrolled.fd");
    let list = pesign::keys::esl::list(&cert);
    fs::write(
        &enrolled,
        pesign::keys::ovmf::add_db(&ms, &list).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let plain = run.file("sb-plain.fd");
    fs::write(&plain, &ms).map_err(|e| e.to_string())?;

    // Enrolled: it starts, and says Secure Boot is on.
    let mut guest = run.boot(&launch(run, &disk, &enrolled))?;
    let deadline = run.deadline() + Duration::from_secs(30);
    guest.seen("Secure Boot on", deadline)?;
    guest.seen(&posted(&refcode::BASE_READY, ""), deadline)?;
    guest.kill();
    // Not enrolled: refused. The test image, not signed: refused.
    let plain_copy = run.file("sb-plain-copy.fd");
    fs::copy(&plain, &plain_copy).map_err(|e| e.to_string())?;
    refused(
        run,
        &disk,
        &plain_copy,
        "the release, its certificate not enrolled,",
    )?;
    let test = run.file("sb-test.img");
    sparse_copy(run.disk, &test)?;
    let enrolled_again = run.file("sb-enrolled-copy.fd");
    fs::copy(&enrolled, &enrolled_again).map_err(|e| e.to_string())?;
    refused(run, &test, &enrolled_again, "the test image, not signed,")?;
    Ok(format!(
        "the release's UKI signed by {} started under Secure Boot with the certificate enrolled \
         (Secure Boot on, base ready); without it, and the test image's unsigned UKI with it, \
         the firmware refused to start them",
        pesign::keys::fingerprint(&cert)
    ))
}
