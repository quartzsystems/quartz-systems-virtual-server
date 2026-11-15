// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The control panel functions, a partition held after a terminal code
//! and dumped, function 22, and dumps: where they are kept, a pool that
//! is full, the limits, the list, the read, the delete.

use std::path::Path;
use std::time::{Duration, Instant};

use model::{IplSource, IplType, Keylock};
use partd::proto::{DumpInfo, PanelArgs};
use partd::{Calls as _, State};
use poold::proto::Calls as _;

use super::{
    attended, history, ipl_patience, qs400_golden, qs400_host, qs400_spec, serial, set_service, up,
    wait_screen,
};
use crate::qemu::image::blank;
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::storage::{disk_names, pool_until, poold};

fn panel(guest: &mut Session, id: u32, function: u8) -> Result<Result<String, String>, String> {
    let answer = partd(guest)?
        .panel(id, function, PanelArgs::default())
        .map_err(|e| e.to_string())?;
    Ok(answer.map_err(|e| manager::refused(&e)))
}

/// Activate with a manual keylock: the IPL is attended, taken through
/// its screens on the console port, to Running.
fn up_manual(guest: &mut Session, id: u32, run: &Run<'_>) -> Result<(), String> {
    manager::activate(guest, id, &qs400_spec(Some(Keylock::Manual)))
        .map_err(|e| manager::refused(&e))?;
    let deadline = ipl_patience(run);
    attended(guest, id, deadline)?;
    manager::until(guest, id, State::Running, deadline)?;
    Ok(())
}

/// Suite 6: 01, 02, 11 to 19 and 20 display what the definition and the
/// current code say; 03, 08, 21 and 22 are refused with the keylock in
/// normal; in manual, 03 restarts the IPL, 08 powers off, and 21 brings
/// dedicated service tools to the console.
pub fn functions(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400_host(run, &[])?;
    let id = manager::partition(&mut guest, "PROD")?;
    up(&mut guest, id, &qs400_spec(None), ipl_patience(run))?;
    let serial_number = said(partd(&mut guest)?.definition(id))?.serial;
    let mut shown = Vec::new();
    let one = panel(&mut guest, id, 1)??;
    if !one.contains("keylock normal") {
        return Err(format!("01: {one}"));
    }
    shown.push(one);
    let eleven = panel(&mut guest, id, 11)??;
    if eleven != "11  C6001090" {
        return Err(format!("11: {eleven}"));
    }
    for f in 12..=19 {
        shown.push(panel(&mut guest, id, f)??);
    }
    let twenty = panel(&mut guest, id, 20)??;
    if !twenty.contains(&serial_number) || !twenty.contains("V1R0M0") {
        return Err(format!("20: {twenty}"));
    }
    shown.push(twenty);
    for f in [3, 8, 21, 22] {
        match panel(&mut guest, id, f)? {
            Err(e) if e.starts_with("QVS2307") => {}
            other => return Err(format!("{f:02} at a normal keylock: {other:?}")),
        }
    }
    // 02: the next activation's IPL, attended at a manual keylock.
    let select = PanelArgs {
        source: Some(IplSource::A),
        ipl_type: Some(IplType::Attended),
        keylock: Some(Keylock::Manual),
    };
    let two = said(partd(&mut guest)?.panel(id, 2, select))?;
    said(partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    said(partd(&mut guest)?.activate(id, String::new(), None, None))?;
    let deadline = ipl_patience(run);
    let options = attended(&mut guest, id, deadline)?;
    qs400_golden("install", "ipl-options", &options, &[4, 5], 0)?;
    manager::until(&mut guest, id, State::Running, deadline)?;
    // 03: the IPL restarted, attended again at the keylock of this activation.
    let before = history(&mut guest, id)?.len();
    panel(&mut guest, id, 3)??;
    let deadline = ipl_patience(run);
    attended(&mut guest, id, deadline)?;
    manager::until(&mut guest, id, State::Running, deadline)?;
    let restarted = history(&mut guest, id)?
        .iter()
        .skip(before)
        .any(|p| p.code == refcode::PARTITION_ACTIVATING.code);
    if !restarted {
        return Err("03 did not restart the IPL".into());
    }
    // 21: dedicated service tools on the console.
    let twentyone = panel(&mut guest, id, 21)??;
    let (_, dst) = wait_screen(
        &mut guest,
        id,
        &["Dedicated Service Tools (DST) Sign On"],
        Instant::now() + Duration::from_secs(120),
    )?;
    qs400_golden("dst", "signon", &dst, &[], 0)?;
    // 08: off.
    panel(&mut guest, id, 8)??;
    let s = manager::ended(&mut guest, id, run.deadline())?;
    if s.state != State::NotActivated {
        return Err(format!("after 08: {}", s.state.word()));
    }
    Ok(format!(
        "keylock normal: `{}`, 11 `C6001090`, `{}`; 03, 08, 21, 22 refused (QVS2307); `{two}`; \
         in manual the IPL attended on the console port (IPL Options as QS/400's golden); 03 \
         restarted the IPL; 21 `{twentyone}` and DST's sign-on as QS/400's golden; 08 powered off",
        shown[0],
        shown.last().cloned().unwrap_or_default()
    ))
}

/// The pool POOL1 on the disk of serial `QSVSUNIT<n>`, designated as
/// the media repository: where it is mounted.
fn repository(guest: &mut Session, n: usize, run: &Run<'_>) -> Result<String, String> {
    let names = disk_names(guest, n + 1)?;
    crate::suite::storage::said(poold(guest)?.create_pool(
        "POOL1".into(),
        "none".into(),
        vec![names[n].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    let p = pool_until(guest, "POOL1", run.deadline(), |p| !p.mount.is_empty())?;
    crate::suite::storage::said(poold(guest)?.designate("POOL1".into()))?;
    Ok(p.mount)
}

/// Wait until the partition's dumps have `name`; it.
fn dump_named(
    guest: &mut Session,
    id: u32,
    name: &str,
    deadline: Instant,
) -> Result<DumpInfo, String> {
    loop {
        if let Some(d) = said(partd(guest)?.dumps(id))?
            .into_iter()
            .find(|d| d.name == name)
        {
            return Ok(d);
        }
        if Instant::now() >= deadline {
            return Err(format!("no dump {name}"));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// A dump's file as the harness sees it: copied out of the host through
/// a scratch disk, its first `bytes`.
fn copy_out(
    guest: &mut Session,
    from: &str,
    scratch_device: &str,
    scratch: &Path,
    bytes: u64,
    to: &Path,
) -> Result<(), String> {
    super::ask(guest, &format!("copy {from} 0 {scratch_device} 0 {bytes}"))?;
    let mut src = std::fs::File::open(scratch).map_err(|e| e.to_string())?;
    let mut dst = std::fs::File::create(to).map_err(|e| e.to_string())?;
    std::io::copy(&mut std::io::Read::take(&mut src, bytes), &mut dst)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Suite 9: a terminal code: the partition is held in Error with that
/// code and its memory kept; function 34 writes a dump; its header is
/// right, its size far below the partition's memory, and a standard
/// tool that reads ELF cores accepts it.
pub fn terminal(run: &Run<'_>) -> Result<String, String> {
    let pool = run.file("pool0.img");
    blank(&pool, 6 << 30)?;
    let scratch = run.file("scratch.img");
    blank(&scratch, 3 << 30)?;
    let mut guest = qs400_host(run, &[pool, scratch.clone()])?;
    let mount = repository(&mut guest, 1, run)?;
    let scratch_device = format!("/dev/{}", disk_names(&mut guest, 3)?[2]);
    let id = manager::partition(&mut guest, "PROD")?;
    up_manual(&mut guest, id, run)?;
    let serial_number = said(partd(&mut guest)?.definition(id))?.serial;
    serial(&mut guest, id, "dbgkill JOB(*STG)")?;
    let s = manager::until(
        &mut guest,
        id,
        State::Error,
        Instant::now() + Duration::from_secs(120),
    )?;
    if s.code != "B1001060" || !s.service.kept {
        return Err(format!("held: {} {:?}", s.code, s.service));
    }
    let text = panel(&mut guest, id, 34)??;
    let name = text
        .split_whitespace()
        .find(|w| w.ends_with(dump::SUFFIX))
        .ok_or_else(|| format!("34: {text}"))?
        .to_string();
    let d = dump_named(
        &mut guest,
        id,
        &name,
        Instant::now() + Duration::from_secs(600),
    )?;
    let memory = super::QS400_MEMORY_MIB << 20;
    if d.allocated >= memory / 2 || d.bytes < memory {
        return Err(format!(
            "the dump is {} bytes and takes {}",
            d.bytes, d.allocated
        ));
    }
    let local = run.file("dump.core");
    let from = format!("{mount}/dumps/{id}/{name}");
    copy_out(
        &mut guest,
        &from,
        &scratch_device,
        &scratch,
        d.bytes,
        &local,
    )?;
    let header = dump::read_header(&mut std::fs::File::open(&local).map_err(|e| e.to_string())?)
        .map_err(|e| format!("the dump's header: {e}"))?;
    if (
        header.partition.as_str(),
        header.serial.as_str(),
        header.code.as_str(),
        header.release.as_str(),
    ) != ("PROD", serial_number.as_str(), "B1001060", "V1R0M0")
    {
        return Err(format!("the header: {header:?}"));
    }
    let readelf = std::process::Command::new("readelf")
        .args(["-h", "-l", "-n"])
        .arg(&local)
        .output()
        .map_err(|e| format!("readelf: {e}"))?;
    let out = String::from_utf8_lossy(&readelf.stdout);
    if !readelf.status.success()
        || !out.contains("CORE (Core file)")
        || !out.contains("NT_PRSTATUS")
    {
        return Err(format!(
            "readelf: {out}{}",
            String::from_utf8_lossy(&readelf.stderr)
        ));
    }
    let loads = out
        .lines()
        .filter(|l| l.trim_start().starts_with("LOAD"))
        .count();
    let still = manager::status(&mut guest, id)?;
    said(partd(&mut guest)?.shut_down(id))?;
    manager::ended(&mut guest, id, run.deadline())?;
    let _ = std::fs::remove_file(&local);
    Ok(format!(
        "dbgkill JOB(*STG): B1001060, the partition held in Error with it current and its memory \
         kept; 34 wrote {name}: {} bytes, {} taken of {} MiB of memory; its header names PROD, \
         {serial_number}, B1001060 and V1R0M0; readelf takes it: an x86-64 core, {loads} \
         segments of memory and a prstatus note a processor; still {} after the dump",
        d.bytes,
        d.allocated,
        super::QS400_MEMORY_MIB,
        still.state.word()
    ))
}

/// Suite 10: function 22 on a running system: a dump is written and the
/// IPL restarts.
pub fn dump_restart(run: &Run<'_>) -> Result<String, String> {
    let pool = run.file("pool0.img");
    blank(&pool, 6 << 30)?;
    let mut guest = qs400_host(run, &[pool])?;
    repository(&mut guest, 1, run)?;
    let id = manager::partition(&mut guest, "PROD")?;
    up_manual(&mut guest, id, run)?;
    let before = history(&mut guest, id)?.len();
    let text = panel(&mut guest, id, 22)??;
    let name = text
        .split_whitespace()
        .find(|w| w.ends_with(dump::SUFFIX))
        .ok_or_else(|| format!("22: {text}"))?
        .to_string();
    let d = dump_named(
        &mut guest,
        id,
        &name,
        Instant::now() + Duration::from_secs(600),
    )?;
    let deadline = ipl_patience(run);
    attended(&mut guest, id, deadline)?;
    manager::until(&mut guest, id, State::Running, deadline)?;
    let h = history(&mut guest, id)?;
    let after: Vec<&str> = h.iter().skip(before).map(|p| p.code.as_str()).collect();
    let written = after
        .iter()
        .position(|c| *c == refcode::PARTITION_DUMP_WRITTEN.code);
    let again = after
        .iter()
        .rposition(|c| *c == refcode::PARTITION_ACTIVATING.code);
    if !matches!((written, again), (Some(w), Some(a)) if w < a) {
        return Err(format!("after 22: {after:?}"));
    }
    Ok(format!(
        "22 on a running system: {name} written ({} bytes, {} taken), then the IPL restarted \
         and completed",
        d.bytes, d.allocated
    ))
}

/// Suite 11: dumps: no repository refuses by name; a full pool leaves no
/// partial file; the limit removes the oldest; list, read and delete.
pub fn dumps(run: &Run<'_>) -> Result<String, String> {
    let pool = run.file("pool0.img");
    blank(&pool, 4 << 30)?;
    let mut guest = qs400_host(run, &[pool])?;
    let id = manager::partition(&mut guest, "PROD")?;
    up(&mut guest, id, &qs400_spec(None), ipl_patience(run))?;
    let mut seen = Vec::new();
    match partd(&mut guest)?
        .take_dump(id)
        .map_err(|e| e.to_string())?
    {
        Err(e) if e[0].id == "QVS2310" => seen.push(e[0].to_string()),
        other => return Err(format!("with no repository: {other:?}")),
    }
    let mount = repository(&mut guest, 1, run)?;
    // The pool filled by a thick virtual disk: the dump fails, nothing left.
    let p = pool_until(&mut guest, "POOL1", run.deadline(), |_| true)?;
    let free_mib = (p.bytes - p.allocated) >> 20;
    crate::suite::storage::said(poold(&mut guest)?.create_vdisk(
        "POOL1".into(),
        "FILL".into(),
        free_mib.saturating_sub(32),
        true,
    ))?;
    said(partd(&mut guest)?.take_dump(id))?;
    guest.seen(
        &posted(&refcode::PARTITION_DUMP_FAILED, ""),
        Instant::now() + Duration::from_secs(600),
    )?;
    let dir = format!("{mount}/dumps/{id}");
    let left = channel(&mut guest)?.ls(&dir).unwrap_or_default();
    if !left.is_empty() {
        return Err(format!("left in {dir}: {left:?}"));
    }
    if manager::status(&mut guest, id)?.state != State::Running {
        return Err("the partition is not running after the dump that failed".into());
    }
    crate::suite::storage::said(poold(&mut guest)?.delete_vdisk(
        "POOL1".into(),
        "FILL".into(),
        true,
    ))?;
    // Two kept at most: the third removes the first.
    set_service(&mut guest, |s| s.max_dumps = 2)?;
    let mut names = Vec::new();
    for _ in 0..3 {
        let name = said(partd(&mut guest)?.take_dump(id))?;
        dump_named(
            &mut guest,
            id,
            &name,
            Instant::now() + Duration::from_secs(600),
        )?;
        names.push(name);
        // Each a second apart at least: their names are their times.
        std::thread::sleep(Duration::from_secs(2));
    }
    std::thread::sleep(Duration::from_secs(2));
    let list: Vec<String> = said(partd(&mut guest)?.dumps(id))?
        .into_iter()
        .map(|d| d.name)
        .collect();
    if list != names[1..] {
        return Err(format!("the dumps kept: {list:?}, taken {names:?}"));
    }
    if !history(&mut guest, id)?
        .iter()
        .any(|p| p.code == refcode::DUMP_REMOVED.code)
    {
        return Err("no C2009060".into());
    }
    // Read through the socket, as the file is.
    let read = super::ask(&mut guest, &format!("dumpread {id} {}", names[2]))?;
    let (bytes, sum) = channel(&mut guest)?.sum(&format!("{dir}/{}", names[2]))?;
    let want = format!("ok {bytes} {sum}");
    if read.first() != Some(&want) {
        return Err(format!("the read: {read:?}, the file: {want}"));
    }
    said(partd(&mut guest)?.delete_dump(id, names[1].clone()))?;
    let list: Vec<String> = said(partd(&mut guest)?.dumps(id))?
        .into_iter()
        .map(|d| d.name)
        .collect();
    if list != [names[2].clone()] {
        return Err(format!("after the delete: {list:?}"));
    }
    match partd(&mut guest)?
        .delete_dump(id, names[1].clone())
        .map_err(|e| e.to_string())?
    {
        Err(e) if e[0].id == "QVS2312" => {}
        other => return Err(format!("a second delete: {other:?}")),
    }
    Ok(format!(
        "{}; a pool with {} MiB free: A2005180 and nothing left in {dir}, the partition running; \
         with two kept, the third dump removed the first (C2009060); read through the socket \
         ({bytes} bytes, the file's sum), deleted, and a second delete refused (QVS2312)",
        seen.join("; "),
        32
    ))
}
