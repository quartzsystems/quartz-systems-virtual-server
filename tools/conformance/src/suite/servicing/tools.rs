// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 12: Service Tools option 8 at the serial port. Every panel
//! against its golden file; a problem closed at the host; a host dump
//! taken and copied to a removable volume; an update installed from
//! one; the code levels; option 7's problem log.

use std::time::{Duration, Instant};

use crate::qemu::term::key;
use crate::suite::base::{Run, channel};
use crate::suite::install::{PASSWORD, USER, golden, option, sign_on, titled};

use super::{Opts, host, other_release};

/// A screen with what changes from run to run masked: digits on a line
/// that has a time, a size, a build, a dump's name or a percentage;
/// a build ID's hexadecimal.
fn masked(screen: &str) -> String {
    screen
        .lines()
        .map(|l| {
            // A build ID is the commit's: `<build>`, as the status
            // screen's golden has it.
            let l = &match l.split_once("build ") {
                Some((before, after)) => {
                    let rest = after.split_once(' ').map_or("", |(_, r)| r);
                    let gap = if rest.is_empty() { "" } else { " " };
                    format!("{before}build <build>{gap}{rest}")
                }
                None => l.to_string(),
            };
            let varies = [
                ":", "bytes", "MiB", "KiB", "GiB", " B ", "build", "HOSTDUMP", "%", "update-",
            ]
            .iter()
            .any(|w| l.contains(w));
            if varies {
                l.chars()
                    .map(|c| if c.is_ascii_digit() { '9' } else { c })
                    .collect()
            } else {
                l.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Hold a screen, masked, against its golden file.
fn held(name: &str, screen: &str) -> Result<(), String> {
    golden(name, &masked(screen))
}

/// F12 until the screen titled `title`: a panel may have shown
/// another over itself first.
fn back(g: &mut crate::suite::base::Guest<'_>, title: &str) -> Result<String, String> {
    for _ in 0..4 {
        g.type_keys(key::F12)?;
        if let Ok(s) = titled(g, title, Instant::now() + Duration::from_secs(3)) {
            return Ok(s);
        }
    }
    Err(format!("F12 did not come back to {title}"))
}

/// Keys, then the screen titled `title`.
fn keys(
    g: &mut crate::suite::base::Guest<'_>,
    k: &str,
    title: &str,
    deadline: Instant,
) -> Result<String, String> {
    g.type_keys(k)?;
    titled(g, title, deadline)
}

/// The kernel's name of the removable volume: the disk whose
/// `removable` is 1.
/// USB storage comes some seconds after the base is ready.
fn stick(g: &mut crate::suite::base::Guest<'_>) -> Result<String, String> {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let c = channel(g)?;
        for name in c.ls("/sys/block")? {
            let name = name.trim_end_matches('/').to_string();
            if name.starts_with("sd")
                && c.text(&format!("/sys/block/{name}/removable"))
                    .is_ok_and(|t| t.trim() == "1")
            {
                return Ok(name);
            }
        }
        if Instant::now() >= until {
            return Err("no removable volume".into());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Suite 12.
#[allow(clippy::too_many_lines)] // a panel a step, in the order a person goes
pub fn tools(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let bytes = std::fs::read(&image).map_err(|e| e.to_string())?;
    let name = "QSVS-NEXT.UPD";
    let usb = run.file("stick.img");
    let fat = diskimg::fat::build(
        &diskimg::fat::fat32_for(bytes.len() as u64 + (64 << 20)),
        &[(name, &bytes)],
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(&usb, fat).map_err(|e| e.to_string())?;
    let mut rig = host(
        run,
        &Opts {
            usb: Some(usb.clone()),
            hooks: vec![qsvshook::FLAKY_FAILS],
            ..Opts::default()
        },
    )?;
    let g = &mut rig.q.guest;
    let deadline = run.deadline();
    let wait = || Instant::now() + Duration::from_mins(3);
    let device = stick(g)?;
    // The flaky service's problem is there to be closed.
    std::thread::sleep(Duration::from_secs(40));
    sign_on(g, USER, PASSWORD, deadline)?;
    let menu = option(g, 8, "Service Functions", deadline)?;
    held("tools-service", &menu)?;
    // 1: problems, one shown, closed at the host.
    let list = option(g, 1, "Work with Problems", deadline)?;
    if !list.contains(refcode::SERVICE_STOPPED.code) {
        return Err(format!("no open problem of the stopped service:\n{list}"));
    }
    held("tools-problems", &list)?;
    let shown = keys(g, &format!("5{}", key::ENTER), "Display Problem", deadline)?;
    held("tools-problem", &shown)?;
    let close = keys(g, key::F10, "Close Problem", deadline)?;
    held("tools-problem-close", &close)?;
    g.type_keys(&format!("Y{}", key::ENTER))?;
    g.screen("the problem closed", wait(), |s| s.contains("is closed."))?;
    // Back to option 8.
    back(g, "Service Functions")?;
    // 2: a host dump.
    let take = option(g, 2, "Take Host Dump", deadline)?;
    held("tools-service-dump", &take)?;
    g.type_keys(key::ENTER)?;
    g.screen("the dump begun", wait(), |s| s.contains("is being taken"))?;
    // 3: the dumps, one copied to the stick.
    let mut dumps = option(g, 3, "Work with Dumps", deadline)?;
    let until = wait();
    while !dumps.contains("HOSTDUMP-") && Instant::now() < until {
        std::thread::sleep(Duration::from_secs(3));
        dumps = keys(g, key::F5, "Work with Dumps", deadline)?;
    }
    held("tools-dumps", &dumps)?;
    let copy = keys(
        g,
        &format!("6{}", key::ENTER),
        "Copy Dump to Removable Volume",
        deadline,
    )?;
    held("tools-dump-copy", &copy)?;
    g.type_keys(&format!("{device}{}{}", key::FIELD_EXIT, key::ENTER))?;
    let copied_to = format!("is copied to {device}.");
    g.screen("the dump copied", wait(), |s| s.contains(&copied_to))?;
    let delete = keys(
        g,
        &format!("4{}", key::ENTER),
        "Confirm Delete of Dump",
        deadline,
    )?;
    held("tools-dump-delete", &delete)?;
    back(g, "Work with Dumps")?;
    back(g, "Service Functions")?;
    // 4: the update from the stick.
    let from = option(g, 4, "Install Update from Removable Volume", deadline)?;
    held("tools-update-device", &from)?;
    let choose = keys(
        g,
        &format!("{device}{}{}", key::FIELD_EXIT, key::ENTER),
        "Select Update Image",
        deadline,
    )?;
    held("tools-update-choose", &choose)?;
    let shown = keys(g, &format!("1{}", key::ENTER), "Update Image", wait())?;
    held("tools-update-image", &shown)?;
    let mut installing = keys(g, key::F10, "Installing Update", deadline)?;
    let until = Instant::now() + Duration::from_mins(5);
    while !installing.contains("Restart now") && Instant::now() < until {
        std::thread::sleep(Duration::from_secs(3));
        installing = keys(g, key::F5, "Installing Update", deadline)?;
    }
    held("tools-update-installing", &installing)?;
    let restart = keys(
        g,
        &format!("1{}", key::ENTER),
        "Confirm Restart into Update",
        deadline,
    )?;
    held("tools-update-restart", &restart)?;
    back(g, "Installing Update")?;
    g.type_keys(&format!("2{}", key::ENTER))?;
    titled(g, "Service Functions", deadline)?;
    // 5: the code levels, with the slot pending.
    let levels = option(g, 5, "Display Code Levels", deadline)?;
    if !levels.contains("pending") {
        return Err(format!("the code levels show nothing pending:\n{levels}"));
    }
    held("tools-levels", &levels)?;
    back(g, "Service Functions")?;
    // 6: shut down all partitions, not done.
    let end = option(g, 6, "Shut Down All Partitions", deadline)?;
    held("tools-service-end", &end)?;
    back(g, "Service Functions")?;
    g.type_keys(key::F3)?;
    // Option 7's problem log.
    option(g, 7, "Display Logs", deadline)?;
    let log = option(g, 3, "Display Problem Log", deadline)?;
    held("tools-log-problems", &log)?;
    g.type_keys(key::F3)?;
    // The stick holds the dump.
    crate::suite::install::power_off(g)?;
    let on_stick = diskimg::fat::list(&std::fs::read(&usb).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let copied = on_stick
        .iter()
        .find(|f| f.starts_with("HOSTDUMP-"))
        .ok_or_else(|| format!("the stick holds {on_stick:?}"))?;
    Ok(format!(
        "option 8's sixteen panels held to their golden files; the stopped service's problem \
         closed at the host; a host dump taken and copied to the removable volume {device} \
         ({copied}); {name} installed from it and the slot pending; the code levels, shutting \
         down every partition and option 7's problem log shown"
    ))
}
