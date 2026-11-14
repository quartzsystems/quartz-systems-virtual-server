// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Service Tools option 2's storage panels: the disk configuration, a
//! pool made from the console, its units shown; each screen held
//! against its golden file.

use std::time::{Duration, Instant};

use poold::proto::{Calls as _, state};

use super::{POOL_DISK_BYTES, pool_disks, pool_until, poold};
use crate::qemu::Drive;
use crate::qemu::term::key;
use crate::suite::base::Run;
use crate::suite::install::{
    Kind, PASSWORD, USER, boot, fill, golden, installed, machine, option, sign_on, titled,
};
use crate::suite::partition::power_off;

const POOLS: &str = "Work with Storage Pools";

/// `pool-tools`.
pub fn tools(run: &Run<'_>) -> Result<String, String> {
    // A copy of this image's host disk, not a disk an earlier run left.
    crate::suite::partition::fresh_host(run)?;
    let disks = installed(run, Kind::Single)?;
    let units = pool_disks(run, 2, POOL_DISK_BYTES)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    launch.keys = true;
    for (i, path) in units.iter().enumerate() {
        launch.extra.push(Drive {
            path: path.clone(),
            serial: format!("QSVSUNIT{i}"),
            place: Some(u8::try_from(4 + i).unwrap_or(4)),
        });
    }
    let mut guest = boot(run, &launch)?;
    let deadline = Instant::now() + 3 * run.timeout;
    while poold(&mut guest)?.pools().is_err() {
        if Instant::now() >= deadline {
            return Err("the pool service does not answer".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    let units_panel = option(&mut guest, 2, "Work with Disk Units", deadline)?;
    golden("tools-disks-others", &units_panel)?;
    let config = option(&mut guest, 6, "Display Disk Configuration", deadline)?;
    golden("tools-disk-config", &config)?;
    if config.matches("Not configured").count() != 2 {
        return Err(format!(
            "the configuration does not show two free disks:\n{config}"
        ));
    }
    guest.type_keys(key::F12)?;
    titled(&mut guest, "Work with Disk Units", deadline)?;

    let empty = option(&mut guest, 7, POOLS, deadline)?;
    golden("tools-pools-empty", &empty)?;
    guest.type_keys(key::F6)?;
    let form = titled(&mut guest, "Create Storage Pool", deadline)?;
    golden("tools-pool-create", &form)?;
    guest.type_keys(&format!(
        "{}{}",
        fill(&[Some("TOOLS1"), None], &[10, 8]),
        key::ENTER
    ))?;
    let pick = guest.screen("the disks of the pool to be made", deadline, |grid| {
        grid.contains("type 1 by each member")
    })?;
    golden("tools-pool-disks", &pick)?;
    guest.type_keys(&format!("11{}", key::ENTER))?;
    let confirm = titled(&mut guest, "Confirm Create of Pool", deadline)?;
    golden("tools-pool-confirm", &confirm)?;
    // A wrong word does nothing.
    guest.type_keys(&format!("TOOLS2{}", key::ENTER))?;
    guest.screen("the word refused", deadline, |grid| {
        grid.message().starts_with("Type TOOLS1 to confirm")
    })?;
    if poold(&mut guest)?
        .pools()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|p| p.name == "TOOLS1")
    {
        return Err("a pool was made by the wrong word".into());
    }
    guest.type_keys(key::F12)?;
    guest.screen("the disks again", deadline, |grid| {
        grid.contains("type 1 by each member")
    })?;
    guest.type_keys(&format!("11{}", key::ENTER))?;
    titled(&mut guest, "Confirm Create of Pool", deadline)?;
    guest.type_keys(&format!("TOOLS1{}", key::ENTER))?;
    titled(&mut guest, POOLS, deadline)?;
    let made = pool_until(&mut guest, "TOOLS1", run.deadline(), |p| {
        p.state == state::ACTIVE && p.work.is_empty()
    })?;
    guest.type_keys(key::F5)?;
    let listed = guest.screen("the new pool, listed", deadline, |grid| {
        grid.contains("TOOLS1") && grid.contains("Active")
    })?;
    golden("tools-pools", &listed)?;
    guest.type_keys(&format!("5{}", key::ENTER))?;
    let status = titled(&mut guest, "Display Storage Pool", deadline)?;
    golden("tools-pool-status", &status)?;
    guest.type_keys(key::ENTER)?;
    titled(&mut guest, POOLS, deadline)?;
    power_off(&mut guest, Duration::from_secs(60))?;
    Ok(format!(
        "option 2 shows the two free disks and option 6 what each is; option 7, F6 and the \
         two disks chosen made TOOLS1 ({}, {} bytes) once its name was typed, and not on a \
         wrong word; option 5 by it shows its two units; golden files tools-disks-others, \
         tools-disk-config, tools-pools-empty, tools-pool-create, tools-pool-disks, \
         tools-pool-confirm, tools-pools, tools-pool-status",
        made.protection, made.bytes
    ))
}
