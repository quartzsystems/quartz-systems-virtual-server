// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The memory a partition leaves is not the next one's to read
//! (`docs/SECURITY.md`). The host's huge pages are exactly one
//! partition's memory, so the second partition is given the first's
//! pages. The first's system fills its memory with a marker (the soak's
//! `fill`), and its dump shows the marker there: the check can see it.
//! It ends; the second starts on the same pages and its dump, its whole
//! memory as the host maps it, holds none of the marker.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use partd::{Calls as _, State};
use poold::proto::Calls as _;
use qsvshook::soak::RESIDUE;

use crate::qemu::image::{blank, sparse_copy};
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::partition::{host, launch, profile};
use crate::suite::storage::{disk_names, pool_until, poold};

/// Occurrences of the marker in the file at `path`.
fn residue_in(path: &Path) -> Result<u64, String> {
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buffer = vec![0u8; 4 << 20];
    let mut carry: Vec<u8> = Vec::new();
    let mut found = 0u64;
    loop {
        let n = f.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(found);
        }
        carry.extend_from_slice(&buffer[..n]);
        found += carry
            .windows(RESIDUE.len())
            .filter(|w| *w == RESIDUE)
            .count() as u64;
        let keep = carry.len().saturating_sub(RESIDUE.len() - 1);
        carry.drain(..keep);
    }
}

/// Dump partition `id`, copy the dump out through the scratch disk and
/// delete it from the pool: the copy's path.
fn dumped(
    guest: &mut Session,
    id: u32,
    mount: &str,
    scratch: (&str, &Path),
    to: PathBuf,
) -> Result<PathBuf, String> {
    let name = said(partd(guest)?.take_dump(id))?;
    let deadline = Instant::now() + Duration::from_mins(10);
    let info = loop {
        if let Some(d) = said(partd(guest)?.dumps(id))?
            .into_iter()
            .find(|d| d.name == name)
        {
            break d;
        }
        if Instant::now() >= deadline {
            return Err(format!("{name} was not written"));
        }
        std::thread::sleep(Duration::from_secs(2));
    };
    let from = format!("{mount}/dumps/{id}/{name}");
    crate::suite::service::ask(
        guest,
        &format!("copy {from} 0 {} 0 {}", scratch.0, info.bytes),
    )?;
    let mut src = File::open(scratch.1).map_err(|e| e.to_string())?;
    let mut dst = File::create(&to).map_err(|e| e.to_string())?;
    std::io::copy(&mut Read::take(&mut src, info.bytes), &mut dst).map_err(|e| e.to_string())?;
    let _ = said(partd(guest)?.delete_dump(id, name));
    Ok(to)
}

/// The pool for dumps on the host's first disk, designated the media
/// repository: its mount, and the host's names of the first two disks.
fn repository(guest: &mut Session, run: &Run<'_>) -> Result<(String, Vec<String>), String> {
    let names = disk_names(guest, 2)?;
    crate::suite::storage::said(poold(guest)?.create_pool(
        "DUMPS".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    let mount = pool_until(guest, "DUMPS", run.deadline(), |p| !p.mount.is_empty())?.mount;
    crate::suite::storage::said(poold(guest)?.designate("DUMPS".into()))?;
    Ok((mount, names))
}

/// The suite.
pub fn clean(run: &Run<'_>) -> Result<String, String> {
    let pool = run.file("clean-pool.img");
    blank(&pool, 8 << 30)?;
    let scratch = run.file("clean-scratch.img");
    blank(&scratch, 4 << 30)?;
    let first = run.file("clean-first.img");
    let second = run.file("clean-second.img");
    sparse_copy(run.disk, &first)?;
    sparse_copy(run.disk, &second)?;
    let fill = run.file("clean-fill.img");
    blank(&fill, 64 << 20)?;
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(&fill)
            .map_err(|e| e.to_string())?;
        f.write_all(b"QSVS-SOAK fill\n")
            .map_err(|e| e.to_string())?;
    }
    crate::suite::partition::fresh_host(run)?;
    let units = vec![pool, scratch.clone(), first, second, fill];
    let mut l = launch(run, &units)?;
    // Two GiB for the host, the rest one partition's pages.
    l.memory_mb = 4608;
    let mut guest = host(run, &l)?;
    let (mount, names) = repository(&mut guest, run)?;
    let scratch_device = format!("/dev/{}", names[1]);
    let memory = partd(&mut guest)?
        .resources()
        .map_err(|e| e.to_string())?
        .memory;
    if memory.available != memory.configurable {
        return Err(format!("the host's memory is not all free: {memory:?}"));
    }
    let mib = memory.configurable;

    // The first: its memory filled with the marker, which its dump shows.
    let a = manager::partition(&mut guest, "FILLED")?;
    let mut spec = profile(1, mib, "side a", "/dev/vdd");
    spec.name = "FILLED".into();
    spec.backing.push("/dev/vdf".into());
    manager::activate(&mut guest, a, &spec).map_err(|e| manager::refused(&e))?;
    let filled = guest.line_with(
        "FILLED| SOAK filled",
        run.deadline() + Duration::from_mins(4),
    )?;
    let a_dump = dumped(
        &mut guest,
        a,
        &mount,
        (&scratch_device, &scratch),
        run.file("clean-filled.core"),
    )?;
    let in_a = residue_in(&a_dump)?;
    if in_a == 0 {
        return Err("the first partition's dump holds no marker: the check cannot see one".into());
    }
    said(partd(&mut guest)?.shut_down(a))?;
    manager::ended(&mut guest, a, Instant::now() + Duration::from_mins(2))?;

    // The second, on the same pages: none of the marker.
    let b = manager::partition(&mut guest, "NEXT")?;
    let mut spec = profile(1, mib, "side a", "/dev/vde");
    spec.name = "NEXT".into();
    manager::activate(&mut guest, b, &spec).map_err(|e| manager::refused(&e))?;
    let deadline = Instant::now() + Duration::from_mins(3);
    loop {
        let s = manager::status(&mut guest, b)?;
        if matches!(s.state, State::Starting | State::Running) && s.current.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("NEXT is {}", s.state.word()));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let b_dump = dumped(
        &mut guest,
        b,
        &mount,
        (&scratch_device, &scratch),
        run.file("clean-next.core"),
    )?;
    let in_b = residue_in(&b_dump)?;
    let reserved = channel(&mut guest)?
        .text("/sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages")
        .unwrap_or_default();
    if in_b != 0 {
        return Err(format!(
            "the second partition's memory holds the first's marker {in_b} times"
        ));
    }
    Ok(format!(
        "{} huge pages, all of them each partition's {mib} MiB; FILLED's system said `{}`, and \
         its dump holds the marker {in_a} times; NEXT, on the same pages after it, holds it \
         nowhere in its whole memory",
        reserved.trim(),
        filled
            .split_once("| ")
            .map_or(filled.as_str(), |(_, t)| t)
            .trim()
    ))
}
