// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The console port, as QS/MC's viewer will have it: the sign-on display
//! against QS/400's golden screen, a command run by keys, a viewer
//! detached and attached, a second refused and one that takes over; the
//! installer on the console port; Service Tools' list of partitions with
//! the system's own code, live.

use std::time::{Duration, Instant};

use model::Keylock;
use model::profile::{IplSource, IplType, MediaDevice};
use partd::Calls as _;
use poold::proto::Calls as _;
use qsipart::console::Key;

use super::{
    ipl_patience, key, keys, qs400_golden, qs400_host, qs400_spec, sign_on, typed, up, view,
    wait_screen,
};
use crate::qemu::image::blank;
use crate::suite::base::{Run, channel};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::storage::{POOL_DISK_BYTES, disk_names, pool_until, poold, qs400_iso};

/// Suite 7: the sign-on display captured as a frame matches QS/400's
/// golden screen; keys sign on and run a command; detached and attached
/// mid-session a viewer gets a whole frame; a second viewer is refused
/// and can take over.
pub fn console(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    up(&mut guest, id, &qs400_spec(None), ipl_patience(run))?;
    let deadline = Instant::now() + Duration::from_secs(180);
    let (_, signon) = wait_screen(&mut guest, id, &["Sign On"], deadline)?;
    // The long password form at QPWDLVL 2 or 3, as QS/400's step 196 has it.
    let long = signon
        .lines()
        .nth(9)
        .is_some_and(|l| l.contains("Program/procedure"));
    let golden = if long { "sign-on-pwdlvl2" } else { "sign-on" };
    qs400_golden("signon", golden, &signon, &[], 12)?;
    sign_on(&mut guest, id, deadline)?;
    let mut k = typed("DSPLIBL");
    k.push(key(Key::Enter));
    keys(&mut guest, id, &k)?;
    let (_, libraries) = wait_screen(&mut guest, id, &["Library List"], deadline)?;
    // Detached, attached again: one whole frame, the screen as it is.
    let viewed = view(&mut guest, id, "view", 2000, &[])?;
    let again = viewed.rows.join("\n");
    if viewed.fact("frames") != "1" || again.trim_end() != libraries.trim_end() {
        return Err(format!(
            "attached again: {} frames\n{again}\n---\n{libraries}",
            viewed.fact("frames")
        ));
    }
    let second = view(&mut guest, id, "second", 1000, &[])?;
    if second.fact("second") != "QVS2314" || second.fact("takeover") != "frame" {
        return Err(format!("a second viewer: {:?}", second.facts));
    }
    if !second.fact("ended").contains("took the console over") {
        return Err(format!(
            "the first viewer was told: {:?}",
            second.fact("ended")
        ));
    }
    // Back to the menu for whoever comes next.
    keys(&mut guest, id, &[key(Key::Function(12))])?;
    Ok(format!(
        "the sign-on display as QS/400's golden signon/{golden} (12 rows); QSECOFR signed on and \
         DSPLIBL run by keys from the viewer; attached again, one whole frame equal to the \
         screen; a second viewer refused (QVS2314) and one that took over got a whole frame, \
         the first told `{}`",
        second.fact("ended")
    ))
}

/// The installed system's sign-on on the console port, the install's
/// codes through the channel before it: how many the system posted.
fn installed_sign_on(
    guest: &mut crate::qemu::session::Session,
    id: u32,
    golden: &str,
    first: &str,
) -> Result<usize, String> {
    // The system's name is the response file's.
    qs400_golden("signon", golden, first, &[2], 12).map_err(|e| format!("{e}\n{first}"))?;
    let codes: Vec<String> = super::history(guest, id)?
        .into_iter()
        .filter(|p| p.source == partd::proto::Source::System)
        .map(|p| p.code)
        .collect();
    for want in ["A6005008", "C6004001", "C6004090", "C6004190", "C6001090"] {
        if !codes.iter().any(|c| c == want) {
            return Err(format!("the system did not post {want}: {codes:?}"));
        }
    }
    Ok(codes.len())
}

/// Suite 8: during an IPL from source D the installer's screen is on
/// the console port; at the pin, the installed system's, after it.
pub fn install(run: &Run<'_>) -> Result<String, String> {
    crate::suite::partition::fresh_host(run)?;
    let iso = qs400_iso()?;
    let pool = run.file("pool0.img");
    blank(&pool, 4 * POOL_DISK_BYTES + (2 << 30))?;
    let mut guest = crate::suite::storage::host(run, &[pool, iso])?;
    let names = disk_names(&mut guest, 2)?;
    crate::suite::storage::said(poold(&mut guest)?.create_pool(
        "POOL1".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(&mut guest, "POOL1", run.deadline(), |p| !p.mount.is_empty())?;
    crate::suite::storage::said(poold(&mut guest)?.designate("POOL1".into()))?;
    channel(&mut guest)?.ask_within(
        &format!("upload QS400 {}", names[1]),
        Duration::from_mins(15),
    )?;
    crate::suite::storage::said(poold(&mut guest)?.create_vdisk(
        "POOL1".into(),
        "PRODLS".into(),
        8192,
        false,
    ))?;
    let id = manager::partition(&mut guest, "PROD")?;
    let mut spec = qs400_spec(Some(Keylock::Manual));
    spec.backing = vec!["POOL1/PRODLS".into()];
    spec.ipl_type = IplType::Attended;
    spec.name = "INSTALL".into();
    let mut profile = spec.profile();
    profile.optical = vec![MediaDevice {
        slot: 3,
        media: Some("QS400".into()),
    }];
    said(partd(&mut guest)?.put_profile(id, profile))?;
    said(partd(&mut guest)?.activate(
        id,
        "INSTALL".into(),
        Some(IplSource::D),
        Some(Keylock::Manual),
    ))?;
    let deadline = ipl_patience(run);
    // An ISO with a response file installs unattended: its status
    // screen; without one, the attended installer's first screen. At
    // the pin, QS/400's installer takes its console mode from a LIC
    // record alone, and a blank disk has none: it draws on serial
    // (A6005008), and the console port first shows the installed
    // system, IPLed again with the mode the host wrote.
    let wanted = [
        "Install Licensed Internal Code - Status",
        "Install Licensed Internal Code",
        "IPL or Install the System",
        "Select a Language",
        "Sign On",
    ];
    let (i, first) = wait_screen(&mut guest, id, &wanted, deadline)?;
    let long = first
        .lines()
        .nth(9)
        .is_some_and(|l| l.contains("Program/procedure"));
    let signon = if long { "sign-on-pwdlvl2" } else { "sign-on" };
    let golden = ["lic-status", "lic-menu", "no-os", "language", signon][i];
    if i == 4 {
        let codes = installed_sign_on(&mut guest, id, golden, &first)?;
        let s = manager::status(&mut guest, id)?;
        said(partd(&mut guest)?.shut_down(id))?;
        manager::ended(&mut guest, id, run.deadline())?;
        return Ok(format!(
            "an IPL from source D with QS/400's ISO in the optical device: QS/400's installer \
             had no console mode and drew on serial (A6005008, see \
             docs/PHASE7.md), its codes through the channel ({} of the system's), then the \
             installed system on the console port as QS/400's golden signon/{golden}; the \
             partition {} with code {}",
            codes,
            s.state.word(),
            s.code
        ));
    }
    if i == 0 {
        // The marks move as it goes: the title, then every step.
        qs400_golden("install", golden, &first, &[], 1).map_err(|e| format!("{e}\n{first}"))?;
        for step in [
            "Partition the load source device",
            "Initialize the LIC partition",
            "Copy the Licensed Internal Code to slot A",
            "Write the LIC record",
        ] {
            if !first.contains(step) {
                return Err(format!("the status screen has no `{step}`:\n{first}"));
            }
        }
    } else {
        qs400_golden("install", golden, &first, &[], 0).map_err(|e| format!("{e}\n{first}"))?;
    }
    let s = manager::status(&mut guest, id)?;
    said(partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    Ok(format!(
        "an IPL from source D with QS/400's ISO in the optical device: the installer's screen \
         `{}` on the console port as QS/400's golden install/{golden}; the partition {} with \
         code {}",
        wanted[i],
        s.state.word(),
        s.code
    ))
}

/// Suite 18: Service Tools option 1 shows the partitions with a live
/// reference code, the system's own, and its release; golden screen.
pub fn tools(run: &Run<'_>) -> Result<String, String> {
    use crate::qemu::term::key as term;
    use crate::suite::install::{
        Kind, PASSWORD, USER, boot, golden, installed, machine, option, sign_on as tools_sign_on,
        titled,
    };
    let image = crate::suite::partition::qs400_image()?;
    let disks = installed(run, Kind::Single)?;
    let copy = run.file("unit.img");
    crate::qemu::image::sparse_copy(&image, &copy)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    launch.keys = true;
    launch.memory_mb = crate::suite::partition::HOST_MEMORY_MB;
    launch.cpus = crate::suite::partition::HOST_CPUS;
    launch.prealloc = true;
    launch.extra.push(crate::qemu::Drive {
        path: copy,
        serial: "QSVSUNIT0".into(),
        place: Some(4),
    });
    let mut guest = boot(run, &launch)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let give_up = run.deadline();
    while partd(&mut guest)?.list().is_err() {
        if Instant::now() >= give_up {
            return Err("the partition manager does not answer".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let id = manager::partition(&mut guest, "PROD")?;
    // The partition's lines would be drawn over the screen.
    channel(&mut guest)?.ask(&format!(
        "{} {} {}",
        qsvshook::proto::command::PUT,
        qsvshook::proto::QUIET,
        hex::encode(b"1")
    ))?;
    tools_sign_on(&mut guest, USER, PASSWORD, deadline)?;
    option(&mut guest, 1, "Work with System Status", deadline)?;
    guest.type_keys(term::F11)?;
    titled(&mut guest, "Display Partitions", deadline)?;
    manager::activate(&mut guest, id, &qs400_spec(None)).map_err(|e| manager::refused(&e))?;
    // The list follows the system's codes as it IPLs.
    let mut codes = Vec::new();
    let until = ipl_patience(run);
    let list = loop {
        let text = guest.grid().text();
        if let Some(row) = text
            .lines()
            .find(|l| l.split_whitespace().nth(1) == Some("PROD"))
        {
            let code = row.split_whitespace().nth(3).unwrap_or("").to_string();
            if codes.last() != Some(&code) {
                codes.push(code);
            }
        }
        if text.contains("Running        C6001090") {
            break text;
        }
        if Instant::now() >= until {
            return Err(format!("the list showed {codes:?}:\n{text}"));
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let system: Vec<&String> = codes
        .iter()
        .filter(|c| c.starts_with("C1") || c.starts_with("C6"))
        .collect();
    if system.len() < 2 {
        return Err(format!("the list showed {codes:?}"));
    }
    golden("tools-partitions-service", &list)?;
    said(partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    guest.type_keys(term::F3)?;
    Ok(format!(
        "option 1's list followed PROD as QS/400 IPLed: {} codes shown in turn ({}), then \
         Running with C6001090 and release V1R1M0; golden file tools-partitions-service",
        codes.len(),
        codes.join(" ")
    ))
}
