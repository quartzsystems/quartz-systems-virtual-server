// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 11 and 12: what the installer will not go on with. Media
//! whose manifest does not verify is refused, and answers that cannot
//! be used stop it, each on a screen that says why and before any byte
//! of any disk is written.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use super::{INSTALL_WITHIN, blank_disks, golden, iso, machine, titled, with_media, with_seed};
use crate::qemu::term::key;
use crate::suite::base::Run;

/// The installer's title.
const TITLE: &str = "Install QS/VS";
/// How the payload's manifest starts.
const MANIFEST_HEAD: &[u8] = b"QSVS-PAYLOAD 1\nproduct ";
/// A line of the payload's manifest.
const MANIFEST_LINE: &[u8] = b"\nbuild ";

/// Whether every byte of `disk` is zero: nothing was written to it.
fn untouched(disk: &Path) -> Result<bool, String> {
    let mut file = fs::File::open(disk).map_err(|e| e.to_string())?;
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return Ok(true),
            Ok(n) if chunk[..n].iter().any(|b| *b != 0) => return Ok(false),
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Suite 11.
pub fn media(run: &Run<'_>) -> Result<String, String> {
    // The ISO with one character of its signed manifest changed.
    let mut image = fs::read(iso(run)?).map_err(|e| e.to_string())?;
    let find = |from: usize, what: &[u8]| {
        image[from..]
            .windows(what.len())
            .position(|w| w == what)
            .map(|at| from + at)
            .ok_or("the ISO has no payload manifest")
    };
    let head = find(0, MANIFEST_HEAD)?;
    let at = find(head, MANIFEST_LINE)? + MANIFEST_LINE.len();
    image[at] ^= 0x01;
    let forged = run.file("forged.iso");
    fs::write(&forged, &image).map_err(|e| e.to_string())?;

    let disks = blank_disks(run, "refused", 1)?;
    let mut launch = with_media(run, machine(run, &[Some(&disks[0])]), false)?;
    launch.cdrom = Some(forged);
    launch.keys = true;
    let mut guest = run.boot(&launch)?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    let shown = guest.screen("the refusal of the media", deadline, |grid| {
        grid.row(1).trim() == TITLE && grid.contains("This installation media is refused")
    })?;
    golden("install-media-refused", &shown)?;
    if !shown.contains("not signed by a key this installer trusts") {
        return Err(format!("the refusal does not say why:\n{shown}"));
    }
    // Enter does not go on to an installation; F3 leaves.
    guest.type_keys(key::ENTER)?;
    std::thread::sleep(std::time::Duration::from_millis(500));
    if !guest.grid().contains("This installation media is refused") {
        return Err("Enter went on from the refusal".into());
    }
    guest.type_keys(key::F3)?;
    guest.wait_exit(deadline)?;
    drop(guest);
    if !untouched(&disks[0])? {
        return Err("the disk was written though the media was refused".into());
    }
    Ok(
        "an ISO whose manifest was changed after it was signed is refused on a screen that says \
         so; Enter installs nothing, F3 leaves; every byte of the disk is still zero"
            .into(),
    )
}

/// Suite 12.
pub fn answers(run: &Run<'_>) -> Result<String, String> {
    let disks = blank_disks(run, "answers", 1)?;
    let seed = super::answers(&["NO-SUCH-DISK"], "", "");
    let mut launch = with_seed(
        run,
        with_media(run, machine(run, &[Some(&disks[0])]), false)?,
        &seed,
    )?;
    launch.keys = true;
    let mut guest = run.boot(&launch)?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    let shown = guest.screen("the stop on the answers", deadline, |grid| {
        grid.row(1).trim() == TITLE && grid.contains("cannot be used")
    })?;
    golden("install-answers-invalid", &shown)?;
    if !shown.contains("there is no disk NO-SUCH-DISK") {
        return Err(format!("the stop does not say what is wrong:\n{shown}"));
    }
    if !untouched(&disks[0])? {
        return Err("the disk was written though the answers were refused".into());
    }
    // From there the operator may install by hand, or leave.
    guest.type_keys(key::ENTER)?;
    guest.screen("the welcome after the stop", deadline, |grid| {
        grid.contains("Press Enter to continue")
    })?;
    guest.type_keys(key::F3)?;
    guest.wait_exit(deadline)?;
    drop(guest);

    // A file that does not parse stops it the same way.
    let mut launch = with_seed(
        run,
        with_media(run, machine(run, &[Some(&disks[0])]), false)?,
        "schema_version = 2\n[install]\ndisk = \"QSVSHD0\"\n",
    )?;
    launch.keys = true;
    let mut guest = run.boot(&launch)?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    let shown = titled_stop(&mut guest, deadline)?;
    if !shown.contains("unknown field") {
        return Err(format!("the stop does not name the unknown key:\n{shown}"));
    }
    drop(guest);
    if !untouched(&disks[0])? {
        return Err("the disk was written though the answers did not parse".into());
    }
    Ok(
        "answers that name a disk the machine does not have, and answers with an unknown key, \
         each stop the installer on a screen that says what is wrong; the disk is untouched; \
         Enter leads to the attended installer"
            .into(),
    )
}

fn titled_stop(
    guest: &mut crate::suite::base::Guest<'_>,
    deadline: Instant,
) -> Result<String, String> {
    titled(guest, TITLE, deadline)?;
    guest.screen("the stop on the answers", deadline, |grid| {
        grid.contains("cannot be used")
    })
}
