// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 8: slot confirmation. Slot B booted as pending confirms and
//! becomes the default in the record and in `BootOrder`; slot B booted
//! with the refuse-to-confirm hook does not, and the next IPL is on
//! slot A with the `A2` code.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use abrec::Slot;
use diskimg::efi;
use hostdisk::layout::{SLOT_A_PATH, SLOT_B_PATH};
use hostdisk::record::HostRecord;
use qsvshook::proto::command;
use release::fact;

use super::{Run, base_ready, channel, fact_of, posted};
use crate::qemu::image::{self, record_of, set_record, sparse_copy};
use crate::qemu::session::Session;
use crate::qemu::{Launch, firmware, vars};

/// The boot entries the harness writes, as an installer would.
const ENTRY_A: u16 = 0;
const ENTRY_B: u16 = 1;

/// Slot B on trial: pending in the record with one attempt, and the
/// firmware's `BootNext`, while `BootOrder` still has slot A alone.
fn on_trial(run: &Run<'_>, name: &str) -> Result<(PathBuf, PathBuf), String> {
    let disk = run.file(&format!("{name}-disk.img"));
    sparse_copy(run.disk, &disk)?;
    set_record(
        &disk,
        HostRecord {
            pending: Some(Slot::B),
            attempts: 1,
            ..HostRecord::INITIAL
        },
    )?;
    let esp = image::esp(&disk)?;
    let template = firmware::locate_vars(&run.base.firmware).map_err(|e| e.to_string())?;
    let template = fs::read(template).map_err(|e| e.to_string())?;
    let store = vars::with_slots(
        &template,
        &efi::load_option("QS/VS slot A", &esp, SLOT_A_PATH),
        &efi::load_option("QS/VS slot B", &esp, SLOT_B_PATH),
        &[ENTRY_A],
        Some(ENTRY_B),
    )?;
    let store_path = run.file(&format!("{name}-vars.fd"));
    fs::write(&store_path, store).map_err(|e| e.to_string())?;
    Ok((disk, store_path))
}

fn launch(run: &Run<'_>, disk: &Path, store: &Path, oem: Vec<String>) -> Launch {
    Launch {
        disk: Some(disk.to_path_buf()),
        vars: Some(store.to_path_buf()),
        snapshot: false,
        oem,
        ..run.launch()
    }
}

/// Power the guest off through the control socket and wait for QEMU.
fn power_off(guest: &mut Session) -> Result<(), String> {
    channel(guest)?.tell(command::POWER_OFF)?;
    guest
        .wait_exit(Instant::now() + Duration::from_secs(30))
        .map(|_| ())
}

fn order_of(store: &Path) -> Result<Vec<u16>, String> {
    let bytes = fs::read(store).map_err(|e| e.to_string())?;
    vars::numbers(&bytes, "BootOrder")?.ok_or_else(|| "the firmware has no BootOrder".to_string())
}

/// The pending slot confirms.
fn confirms(run: &Run<'_>) -> Result<String, String> {
    let (disk, store) = on_trial(run, "confirm")?;
    let mut guest = run.boot(&launch(run, &disk, &store, Vec::new()))?;
    let deadline = run.deadline();
    guest.expect("IPL slot B (qsvs.slot=B)", deadline)?;
    guest.expect(
        &posted(
            &refcode::RECORD_READ,
            "default A, pending B (0 attempts left)",
        ),
        deadline,
    )?;
    base_ready(&mut guest, deadline)?;
    let confirmed = posted(
        &refcode::SLOT_CONFIRMED,
        "slot B is the default; BootOrder 0001,0000",
    );
    guest.expect(&confirmed, deadline)?;
    let status = guest.channel(deadline)?.status()?;
    if fact_of(&status, fact::RECORD)? != "default B, nothing pending" {
        return Err(format!(
            "the status's record is `{}`",
            fact_of(&status, fact::RECORD)?
        ));
    }
    power_off(&mut guest)?;
    drop(guest);

    let (record, _) = record_of(&disk)?;
    let wanted = HostRecord {
        default_slot: Slot::B,
        pending: None,
        attempts: 0,
        confirmed: [None, Some(release::RELEASE)],
    };
    if record != wanted {
        return Err(format!("the record on the disk is {record:?}"));
    }
    let order = order_of(&store)?;
    if order.get(..2) != Some(&[ENTRY_B, ENTRY_A]) {
        return Err(format!("the firmware's BootOrder is {order:04X?}"));
    }
    Ok(format!(
        "slot B on trial confirmed: `{confirmed}`; the record has default B and {} in it",
        release::RELEASE
    ))
}

/// The pending slot does not confirm; the next IPL is back on slot A.
fn falls_back(run: &Run<'_>) -> Result<String, String> {
    let (disk, store) = on_trial(run, "refuse")?;
    let refuse = vec![qsvshook::oem_string(qsvshook::REFUSE_CONFIRM)];
    let mut guest = run.boot(&launch(run, &disk, &store, refuse))?;
    let deadline = run.deadline();
    guest.expect("IPL slot B (qsvs.slot=B)", deadline)?;
    base_ready(&mut guest, deadline)?;
    guest.expect("TEST HOOK: the slot is not confirmed", deadline)?;
    let status = guest.channel(deadline)?.status()?;
    if fact_of(&status, fact::RECORD)? != "default A, pending B (0 attempts left)" {
        return Err(format!(
            "the record after the refused confirmation is `{}`",
            fact_of(&status, fact::RECORD)?
        ));
    }
    if guest.said(&refcode::SLOT_CONFIRMED.tag()) {
        return Err("the slot was confirmed though the hook refused".into());
    }
    power_off(&mut guest)?;
    drop(guest);
    // The firmware may list entries of its own after ours.
    if order_of(&store)?.first() != Some(&ENTRY_A) {
        return Err(format!(
            "BootOrder changed without a confirmation: {:04X?}",
            order_of(&store)?
        ));
    }

    // The firmware used BootNext once: this IPL is on the default.
    let mut guest = run.boot(&launch(run, &disk, &store, Vec::new()))?;
    let deadline = run.deadline();
    guest.expect("IPL slot A (qsvs.slot=A)", deadline)?;
    let fell_back = posted(&refcode::SLOT_FELL_BACK, "slot B; this IPL is on slot A");
    guest.expect(&fell_back, deadline)?;
    base_ready(&mut guest, deadline)?;
    let status = guest.channel(deadline)?.status()?;
    if fact_of(&status, fact::RECORD)? != "default A, nothing pending" {
        return Err(format!(
            "the record after the fall back is `{}`",
            fact_of(&status, fact::RECORD)?
        ));
    }
    power_off(&mut guest)?;
    drop(guest);
    let (record, _) = record_of(&disk)?;
    if (record.default_slot, record.pending) != (Slot::A, None) {
        return Err(format!("the record on the disk is {record:?}"));
    }
    Ok(format!(
        "slot B refused to confirm; the next IPL was on slot A with `{fell_back}`"
    ))
}

/// The suite.
pub fn slot_confirm(run: &Run<'_>) -> Result<String, String> {
    Ok(format!("{}; {}", confirms(run)?, falls_back(run)?))
}
