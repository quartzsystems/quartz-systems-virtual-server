// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The suites of the installer, the mirrored load source and Service
//! Tools. They start from blank disks and the test image's ISO: what
//! they boot afterwards was written by the installer, not by the build.
//!
//! An installation takes a while, so the two the later suites start
//! from (one disk, and two mirrored and in step) are made once a build
//! and kept; each suite works on copies.

mod attended;
mod mirror;
mod pull;
mod refuse;
mod tools;
mod unattended;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hostdisk::gpt;
use hostdisk::layout::{DATA_TYPE_GUID, RECORD_TYPE_GUID, SLOT_A_PATH, SLOT_B_PATH};
use hostdisk::record::HostRecord;
use hostdisk::set::{self, SetLabel};

use super::base::{Guest, Run, SuiteFn, base_ready};
use crate::qemu::term::Grid;
use crate::qemu::{Drive, Launch, image};

/// The suites, in the order of the handoff.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("install-serial", attended::serial),
    ("install-display", attended::display),
    ("install-unattended", unattended::mirrored),
    ("mirror-degraded", mirror::degraded),
    ("mirror-replace", mirror::replace),
    ("install-power-pull", pull::install),
    ("mirror-power-pull", pull::mirror),
    ("storage-kill", mirror::storage_kill),
    ("tools-network", tools::network),
    ("tools-sign-on", tools::sign_on_rules),
    ("media-refused", refuse::media),
    ("answers-invalid", refuse::answers),
    ("installed-base", unattended::installed_base),
];

/// The size of a disk a suite installs on: little more than the least
/// a load source may be, so a synchronization is short.
pub const DISK_BYTES: u64 = 640 * 1024 * 1024;
/// The serial numbers of the two load-source disks.
pub const SERIALS: [&str; 2] = [crate::qemu::DISK_SERIAL, "QSVSHD1"];
/// The serial number of the seed volume's disk.
pub const SEED_SERIAL: &str = "QSVSSEED";
/// The service tools user the suites install.
pub const USER: &str = "ADMIN";
/// That user's password.
pub const PASSWORD: &str = "service tools 1";
/// The host name the suites install.
pub const HOST: &str = "vs-test";
/// What an installation is given, from the firmware to the restart.
pub const INSTALL_WITHIN: Duration = Duration::from_secs(240);
/// What PID 1 says when the installer has ended.
pub const INSTALLER_ENDED: &str = "The installer has ended: restarting";

/// The hash of [`PASSWORD`], as an answers file carries it.
pub fn password_hash() -> String {
    svcusers::hash_password(PASSWORD, *b"conformance-salt")
}

/// An answers file: the disks by serial number, the user, and the
/// keys of the `[host]` and `[network]` tables given.
pub fn answers_with(disks: &[&str], host: &str, network: &str) -> String {
    let disks: Vec<String> = disks.iter().map(|d| format!("\"{d}\"")).collect();
    format!(
        "schema_version = 2\n\n[install]\ndisks = [{}]\nuser_id = \"{USER}\"\n\
         password_hash = \"{}\"\n\n[host]\n{host}\n[network]\n{network}",
        disks.join(", "),
        password_hash()
    )
}

/// An answers file for the host [`HOST`], with further keys of the
/// `[host]` table and the keys of the `[network]` table.
pub fn answers(disks: &[&str], host: &str, network: &str) -> String {
    answers_with(disks, &format!("name = \"{HOST}\"\n{host}"), network)
}

/// Install on a blank `disk` with these keys of the `[host]` and
/// `[network]` tables as answers: what a seed would have brought a
/// host that was not installed.
pub fn configured(run: &Run<'_>, disk: &Path, host: &str, network: &str) -> Result<(), String> {
    image::blank(disk, DISK_BYTES)?;
    let host = if host.contains("name =") {
        host.to_string()
    } else {
        format!("name = \"{HOST}\"\n{host}")
    };
    install_unattended(
        run,
        &[disk],
        &answers_with(&SERIALS[..1], &host, network),
        false,
    )
    .map(|_| ())
}

/// The test image's ISO.
pub fn iso<'a>(run: &Run<'a>) -> Result<&'a Path, String> {
    run.images
        .test_iso
        .as_deref()
        .filter(|p| p.is_file())
        .ok_or_else(|| "the test image's ISO is not built; run `all --test-image`".into())
}

/// Blank disks of this suite, [`DISK_BYTES`] each, named after `tag`.
pub fn blank_disks(run: &Run<'_>, tag: &str, count: usize) -> Result<Vec<PathBuf>, String> {
    (0..count)
        .map(|i| {
            let path = run.file(&format!("{tag}-disk{i}.img"));
            image::blank(&path, DISK_BYTES)?;
            Ok(path)
        })
        .collect()
}

/// The launch of a machine with `disks` as its load-source disks, in
/// their fixed places: a missing one leaves its place empty.
pub fn machine(run: &Run<'_>, disks: &[Option<&Path>]) -> Launch {
    let mut launch = Launch {
        hook: Some(run.file("hook.sock")),
        snapshot: false,
        pinned: true,
        ..run.base.clone()
    };
    for (i, disk) in disks.iter().enumerate() {
        if let Some(disk) = disk {
            launch.extra.push(Drive {
                path: disk.to_path_buf(),
                serial: SERIALS.get(i).copied().unwrap_or("QSVSHDX").to_string(),
                place: Some(u8::try_from(i).unwrap_or(0)),
            });
        }
    }
    launch
}

/// The same machine with the test image's installation media in it,
/// started first.
pub fn with_media(run: &Run<'_>, launch: Launch, usb: bool) -> Result<Launch, String> {
    Ok(with_media_of(launch, iso(run)?, usb))
}

/// The same machine with the media `iso` in it, started first.
pub fn with_media_of(mut launch: Launch, iso: &Path, usb: bool) -> Launch {
    let iso = iso.to_path_buf();
    if usb {
        launch.usb = Some(iso);
    } else {
        launch.cdrom = Some(iso);
    }
    launch.media_first = true;
    launch.hook = None;
    launch
}

/// The same machine with a seed volume holding `seed_toml`.
pub fn with_seed(run: &Run<'_>, mut launch: Launch, seed_toml: &str) -> Result<Launch, String> {
    let path = run.file("seed.img");
    image::seed_volume(&path, release::SEED_LABEL, seed_toml)?;
    launch.extra.push(Drive {
        path,
        serial: SEED_SERIAL.into(),
        place: Some(8),
    });
    Ok(launch)
}

/// Run an unattended installation on `disks` and wait for the restart
/// the installer asks for: the serial log.
pub fn install_unattended(
    run: &Run<'_>,
    disks: &[&Path],
    seed_toml: &str,
    usb: bool,
) -> Result<String, String> {
    install_from(run, iso(run)?, disks, seed_toml, usb)
}

/// [`install_unattended`] from the media `iso`.
pub fn install_from(
    run: &Run<'_>,
    iso: &Path,
    disks: &[&Path],
    seed_toml: &str,
    usb: bool,
) -> Result<String, String> {
    let places: Vec<Option<&Path>> = disks.iter().map(|d| Some(*d)).collect();
    let launch = with_seed(
        run,
        with_media_of(machine(run, &places), iso, usb),
        seed_toml,
    )?;
    let mut guest = run.boot(&launch)?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    guest
        .seen(INSTALLER_ENDED, deadline)
        .map_err(|e| format!("{e}; the terminal shows:\n{}", guest.grid().text()))?;
    guest.wait_exit(deadline)?;
    if !guest.grid().contains("is installed.") && !guest.log.contains("is installed.") {
        return Err("the installer ended without saying the product is installed".into());
    }
    Ok(guest.log.clone())
}

/// Boot the installed machine to "Base ready" with the channel up.
pub fn boot<'a>(run: &Run<'a>, launch: &Launch) -> Result<Guest<'a>, String> {
    let mut guest = run.boot(launch)?;
    let deadline = run.deadline();
    base_ready(&mut guest, deadline)?;
    guest.channel(deadline)?;
    Ok(guest)
}

/// Power the guest off through the channel and wait for QEMU to end.
pub fn power_off(guest: &mut Guest<'_>) -> Result<(), String> {
    use qsvshook::proto::command;
    guest
        .channel(Instant::now() + Duration::from_secs(10))?
        .tell(command::POWER_OFF)?;
    guest.wait_exit(Instant::now() + Duration::from_secs(60))?;
    Ok(())
}

/// Wait until the mirrored load source is whole: both units active and
/// nothing being copied.
pub fn protected(guest: &mut Guest<'_>, within: Duration) -> Result<qsvsstgd::Status, String> {
    let deadline = Instant::now() + within;
    guest
        .channel(deadline)?
        .storage_until("both units in step", deadline, |s| {
            s.protection == qsvsstgd::MIRRORED
                && s.units.len() == 2
                && s.whole()
                && s.work.is_empty()
        })
}

/// What kind of installed machine a suite starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One disk, unprotected.
    Single,
    /// Two disks, mirrored and in step.
    Mirrored,
}

/// Copies, for this suite, of the disks of an installed machine of
/// `kind`. The installation itself is made once for a build's ISO.
pub fn installed(run: &Run<'_>, kind: Kind) -> Result<Vec<PathBuf>, String> {
    let (tag, count) = match kind {
        Kind::Single => ("single", 1),
        Kind::Mirrored => ("mirrored", 2),
    };
    let kept: Vec<PathBuf> = (0..count)
        .map(|i| run.work.join(format!("installed-{tag}-{i}.img")))
        .collect();
    let stamp = run.work.join(format!("installed-{tag}.iso"));
    // The ISO's name is its boot image's hash; the payload beside it
    // can change without that, so its size and time are part of what
    // an installation is kept for.
    let media = iso(run)?;
    let meta = fs::metadata(media).map_err(|e| e.to_string())?;
    let built = format!(
        "{} {} {:?}",
        media.display(),
        meta.len(),
        meta.modified().map_err(|e| e.to_string())?
    );
    let fresh =
        fs::read_to_string(&stamp).is_ok_and(|s| s == built) && kept.iter().all(|p| p.is_file());
    if !fresh {
        let _ = fs::remove_file(&stamp);
        for path in &kept {
            image::blank(path, DISK_BYTES)?;
        }
        let disks: Vec<&Path> = kept.iter().map(PathBuf::as_path).collect();
        install_unattended(run, &disks, &answers(&SERIALS[..count], "", ""), false)?;
        // The first IPL: the identity is made, and the second unit of a
        // mirrored pair is written. Then an orderly power off.
        let places: Vec<Option<&Path>> = disks.iter().map(|d| Some(*d)).collect();
        let mut guest = boot(run, &machine(run, &places))?;
        if kind == Kind::Mirrored {
            protected(&mut guest, Duration::from_secs(180))?;
        }
        power_off(&mut guest)?;
        drop(guest);
        fs::write(&stamp, &built).map_err(|e| e.to_string())?;
    }
    kept.iter()
        .enumerate()
        .map(|(i, from)| {
            let to = run.file(&format!("disk{i}.img"));
            image::sparse_copy(from, &to)?;
            Ok(to)
        })
        .collect()
}

/// What a load-source disk image holds, read from the host's side.
pub struct OnDisk {
    /// Slot A's UKI on the ESP.
    pub slot_a: Vec<u8>,
    /// Slot B's UKI on the ESP.
    pub slot_b: Vec<u8>,
    /// The A/B record and its sequence.
    pub record: (HostRecord, u64),
    /// The mirrored set's label, when there is one.
    pub label: Option<SetLabel>,
    /// The GUID of the data partition.
    pub data_guid: uuid::Uuid,
}

/// Read a load-source disk image.
pub fn on_disk(disk: &Path) -> Result<OnDisk, String> {
    use std::io::{Read, Seek, SeekFrom};
    let text = |e: &dyn std::fmt::Display| format!("{}: {e}", disk.display());
    let mut file = fs::File::open(disk).map_err(|e| text(&e))?;
    let table = gpt::entries(&mut file).map_err(|e| text(&e))?;
    let part = |ty| {
        table
            .iter()
            .find(|e| e.type_guid == ty)
            .copied()
            .ok_or_else(|| text(&format!("no partition of type {ty}")))
    };
    let mut read = |at: u64, len: u64| -> Result<Vec<u8>, String> {
        let mut bytes = vec![0u8; usize::try_from(len).map_err(|e| text(&e))?];
        file.seek(SeekFrom::Start(at)).map_err(|e| text(&e))?;
        file.read_exact(&mut bytes).map_err(|e| text(&e))?;
        Ok(bytes)
    };
    let esp = part(diskimg::layout::ESP_TYPE_GUID)?;
    let esp = read(esp.offset(), esp.bytes())?;
    let records = part(RECORD_TYPE_GUID)?;
    let records = read(records.offset(), records.bytes())?;
    let label_at = usize::try_from(set::LABEL_OFFSET).unwrap_or(0);
    let label = abrec::AbPart::open(
        set::LABEL_SPEC,
        std::io::Cursor::new(records[label_at..label_at + 2 * set::LABEL_SPEC.slot_bytes].to_vec()),
    )
    .read()
    .ok()
    .and_then(|stored| SetLabel::decode(&stored.payload));
    Ok(OnDisk {
        slot_a: diskimg::fat::read_file(&esp, SLOT_A_PATH).map_err(|e| text(&e))?,
        slot_b: diskimg::fat::read_file(&esp, SLOT_B_PATH).map_err(|e| text(&e))?,
        record: hostdisk::record::read_bytes(&records[..2 * hostdisk::record::SPEC.slot_bytes])
            .map_err(|e| text(&e))?,
        label,
        data_guid: part(DATA_TYPE_GUID)?.guid,
    })
}

/// Wait for a screen titled `title` on the serial terminal: its text.
pub fn titled(guest: &mut Guest<'_>, title: &str, deadline: Instant) -> Result<String, String> {
    guest.screen(&format!("the screen `{title}`"), deadline, |grid: &Grid| {
        grid.row(1).trim() == title
    })
}

/// Type the number of a menu option and Enter: the screen titled
/// `title` that follows.
pub fn option(
    guest: &mut Guest<'_>,
    number: u8,
    title: &str,
    deadline: Instant,
) -> Result<String, String> {
    use crate::qemu::term::key;
    guest.type_keys(&format!("{number}{}", key::ENTER))?;
    titled(guest, title, deadline)
}

/// The keys that fill a form's fields, of the lengths `lens`, from the
/// first: `None` leaves a field as it is; a value shorter than its
/// field clears the rest of it. A value that fills its field moves the
/// cursor on by itself.
pub fn fill(values: &[Option<&str>], lens: &[usize]) -> String {
    use crate::qemu::term::key;
    let mut keys = String::new();
    for (value, len) in values.iter().zip(lens) {
        match value {
            None => keys.push_str(key::TAB),
            Some(value) => {
                keys.push_str(value);
                if value.len() < *len {
                    keys.push_str(key::FIELD_EXIT);
                }
            }
        }
    }
    keys
}

/// Hold a screen against its golden file.
pub fn golden(name: &str, screen: &str) -> Result<(), String> {
    crate::qemu::golden::check(name, screen, &release::RELEASE.to_string())
}

/// The title of the sign-on display.
pub const SIGN_ON: &str = "Service Tools Sign On";
/// The title of the main menu.
pub const MAIN_MENU: &str = "QS/VS Service Tools";

/// Press Enter at the serial port and sign on: the main menu's text.
pub fn sign_on(
    guest: &mut Guest<'_>,
    user: &str,
    password: &str,
    deadline: Instant,
) -> Result<String, String> {
    use crate::qemu::term::key;
    guest.type_keys(key::ENTER)?;
    titled(guest, SIGN_ON, deadline)?;
    guest.type_keys(&format!("{user}{}{password}{}", key::TAB, key::ENTER))?;
    titled(guest, MAIN_MENU, deadline)
}

/// Wait until the reference code history has `code` with `detail` in
/// its text. A code posted while a session has the serial port is on
/// the display and in the history, not on the port.
pub fn in_history(
    guest: &mut Guest<'_>,
    code: &refcode::RefCode,
    detail: &str,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        let history = guest
            .channel(deadline)?
            .text("/data/log/refcodes.ndjson")
            .unwrap_or_default();
        if history
            .lines()
            .any(|line| line.contains(code.code) && line.contains(detail))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the history has no `{}` with `{detail}`",
                code.line()
            ));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// `error` with the newest lines of the host's system log under it:
/// what the services said, which the serial port does not show.
pub fn with_log(guest: &mut Guest<'_>, error: String) -> String {
    let log = guest
        .channel(Instant::now() + Duration::from_secs(5))
        .and_then(|c| c.text("/data/log/system.ndjson"))
        .unwrap_or_default();
    let lines: Vec<&str> = log.lines().collect();
    let newest = lines[lines.len().saturating_sub(25)..].join("\n");
    format!("{error}\n--- the system log's newest lines\n{newest}")
}
