// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The cheap guest: QS/VS's own disk image started in a partition. Its
//! UKI is a bzImage of the same kernel family and its PID 1 says on
//! the serial port how far it got.
//!
//! The host would take a disk with QS/VS's partitions for a
//! load-source disk of its own, so the image reaches the host wrapped
//! in a megabyte of zeros, and the host copies it into a file on
//! `/data`: the guest's disk is then a file of the host's, as a
//! virtual disk in a storage pool will be. To read what the guest
//! wrote, its data partition is copied from that file to a blank disk
//! and mounted.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use qsvshook::proto::{command, word};
use state::Inventory;

use super::{
    Counts, QSVS_MEMORY_MIB, ask, counts_in, ended, expect_code, expect_guest, fresh_host, host,
    host_unaffected, launch, pid, profile, signal, start, vmm_line,
};
use crate::qemu::image::{WRAP, blank, data_partition, wrapped};
use crate::qemu::session::Session;
use crate::suite::banner_prefix;
use crate::suite::base::{Guest, Run, posted};

/// The wrapped image on the host.
const WRAPPED: &str = "/dev/vdb";
/// The blank disk the guest's data partition is copied to.
const SCRATCH: &str = "/dev/vdc";
/// The guest's disk: a file of the host's.
const GUEST_FILE: &str = "/data/guest.img";
/// Where the guest's data partition is mounted.
const MOUNT: &str = "/run/guest";
/// How long a copy on the host may take.
const COPY_WITHIN: Duration = Duration::from_secs(120);

/// The cheap guest's place on the host.
struct Cheap {
    /// Bytes of the guest's image.
    image_bytes: u64,
    /// Where its data partition is in the image, and its length.
    data: (u64, u64),
}

fn cheap_profile(processors: u32) -> String {
    profile(processors, QSVS_MEMORY_MIB, "side = \"a\"", GUEST_FILE)
}

/// Boot a host with the wrapped guest image and a blank disk, and
/// copy the image to a file on the host. The partition hook's first
/// start finds no file yet: that it says so is checked on the way.
fn cheap<'a>(run: &Run<'a>, processors: u32) -> Result<(Guest<'a>, Cheap), String> {
    // The release image: a test image's hooks want a second serial
    // port for their channel, which a partition has not.
    let image = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    fresh_host(run)?;
    let container = run.file("wrapped.img");
    let image_bytes = wrapped(image, &container)?;
    let data = data_partition(image)?;
    let scratch = run.file("scratch.img");
    blank(&scratch, data.1)?;
    let units: [PathBuf; 2] = [container, scratch];
    let l = launch(run, &units, &cheap_profile(processors))?;
    let mut guest = host(run, &l)?;
    let deadline = run.deadline();
    // The unit is not there yet.
    let why = expect_code(&mut guest, &refcode::PARTITION_DISK_UNREADABLE, deadline)?;
    if !why.contains(GUEST_FILE) {
        return Err(format!("the first start did not name its unit: `{why}`"));
    }
    expect_code(&mut guest, &refcode::PARTITION_NOT_IPLED, deadline)?;
    copy(&mut guest, WRAPPED, WRAP, GUEST_FILE, 0, image_bytes)?;
    Ok((guest, Cheap { image_bytes, data }))
}

fn copy(
    guest: &mut Session,
    from: &str,
    skip: u64,
    to: &str,
    seek: u64,
    bytes: u64,
) -> Result<(), String> {
    let line = format!("{} {from} {skip} {to} {seek} {bytes}", command::COPY);
    let deadline = Instant::now() + Duration::from_secs(10);
    guest
        .channel(deadline)?
        .ask_within(&line, COPY_WITHIN)
        .map(|_| ())
}

/// A file of the guest's data file system, as the host reads it: the
/// partition copied out of the guest's disk to the blank disk, and
/// mounted there.
fn guest_file(
    guest: &mut Session,
    cheap: &Cheap,
    path: &str,
) -> Result<(String, Vec<String>), String> {
    copy(guest, GUEST_FILE, cheap.data.0, SCRATCH, 0, cheap.data.1)?;
    ask(guest, &format!("{} {SCRATCH} {MOUNT}", command::MOUNT))?;
    let read = (|| {
        let deadline = Instant::now() + Duration::from_secs(10);
        let channel = guest.channel(deadline)?;
        let text = channel.text(&format!("{MOUNT}/{path}"))?;
        let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
        Ok((text, channel.ls(&format!("{MOUNT}/{dir}"))?))
    })();
    ask(guest, &format!("{} {MOUNT}", command::UNMOUNT))?;
    read
}

/// Wait for the guest's own PID 1 to say its base is ready.
fn guest_ready(guest: &mut Session, deadline: Instant) -> Result<(), String> {
    let waited = (|| {
        expect_guest(guest, &posted(&refcode::IPL_STARTED, ""), deadline)?;
        expect_guest(guest, &banner_prefix(), deadline)?;
        expect_guest(guest, &posted(&refcode::BASE_READY, ""), deadline)
    })();
    waited.map_err(|e| format!("{e}; {}", super::diagnosis(guest)))
}

/// End the partition from the host and wait for its end to be posted.
fn end(guest: &mut Session, deadline: Instant) -> Result<(), String> {
    signal(guest, "TERM")?;
    expect_code(guest, &refcode::PARTITION_ENDED, deadline)?;
    match ended(guest, deadline)?.as_str() {
        "exit 50" => Ok(()),
        other => Err(format!(
            "ended by the host, qsvmm gave `{other}`, not `exit 50`"
        )),
    }
}

/// Suite 1: the QS/VS UKI boots as a guest with 1, 2 and 4 processors
/// and prints its banner; the guest counts as many as it was given.
pub fn boot(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, cheap) = cheap(run, 1)?;
    let mut seen = Vec::new();
    for processors in [1u32, 2, 4] {
        let deadline = Instant::now() + 3 * run.timeout;
        let began = Instant::now();
        start(&mut guest, Some(&cheap_profile(processors)))?;
        let built = expect_code(&mut guest, &refcode::PARTITION_MACHINE_BUILT, deadline)?;
        if !built.contains(&format!("{processors} processor(s)")) {
            return Err(format!("{processors} processors asked for: `{built}`"));
        }
        expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
        guest_ready(&mut guest, deadline)?;
        let took = began.elapsed().as_secs_f64();
        // The inventory is written before the base is ready; a moment
        // more has it on the disk.
        std::thread::sleep(Duration::from_secs(1));
        end(&mut guest, deadline)?;
        let (text, _) = guest_file(&mut guest, &cheap, "state/inventory.toml")?;
        let inventory: Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
        if inventory.processors.threads != processors {
            return Err(format!(
                "given {processors} processors, the guest counted {}",
                inventory.processors.threads
            ));
        }
        seen.push(format!("{processors} ({took:.1} s to its base ready)"));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "the QS/VS image, {} bytes as a file of the host, IPLed in a partition with {} \
         processors; the guest's inventory counts as many each time",
        cheap.image_bytes,
        seen.join(", ")
    ))
}

/// Suite 2: virtio-scsi. A guest reads and writes its LUN and what it
/// wrote is on the host's device; its flushes reach the host; requests
/// beyond the unit and requests that are none are refused.
pub fn scsi(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, cheap) = cheap(run, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    start(&mut guest, None)?;
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    guest_ready(&mut guest, deadline)?;
    std::thread::sleep(Duration::from_secs(2));
    end(&mut guest, deadline)?;
    let Counts {
        reads,
        writes,
        flushes,
        refused,
    } = counts_in(&guest.log, 0).ok_or("qsvmm did not report the unit's counts")?;
    if reads == 0 || writes == 0 || flushes == 0 {
        return Err(format!(
            "the guest mounted its data and wrote its state, yet the unit saw {reads} reads, \
             {writes} writes and {flushes} flushes"
        ));
    }
    // What the guest wrote, read by the host from the host's own file.
    let (text, names) = guest_file(&mut guest, &cheap, "state/inventory.toml")?;
    let inventory: Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
    let disk = inventory
        .disks
        .first()
        .ok_or("the guest's inventory has no disk")?;
    if !disk.model.contains("VIRTUAL DISK") {
        return Err(format!("the guest's disk is a `{}`", disk.model));
    }

    // The requests a guest's kernel does not send, of a host device.
    let probe = format!("{} {SCRATCH}", command::SCSI_PROBE);
    let deadline = Instant::now() + Duration::from_secs(10);
    let lines = guest
        .channel(deadline)?
        .ask_within(&probe, Duration::from_secs(60))?;
    let seen: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.strip_prefix(word::LINE))
        .map(str::trim)
        .collect();
    let last = seen.last().copied().unwrap_or_default();
    if !last.contains("no write reached the unit") || seen.len() < 12 {
        return Err(format!("the probe saw {seen:?}"));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "the guest's unit saw {reads} reads, {writes} writes, {flushes} flushes carried out on \
         the host, {refused} commands refused; the host reads the guest's state/ ({}) from its \
         own file, the guest's disk a `{}` serial {}; probe of {SCRATCH}: {} checks, {last}",
        names.join(" "),
        disk.model,
        disk.serial,
        seen.len()
    ))
}

/// A field of `/proc/<pid>/status`.
fn field<'t>(status: &'t str, name: &str) -> Result<&'t str, String> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .ok_or_else(|| format!("the status has no {name}"))
}

/// The identity and capabilities in one task's status.
fn unprivileged(status: &str, what: &str) -> Result<(), String> {
    let id = qsvmm::PARTITION_ID.to_string();
    for name in ["Uid", "Gid"] {
        let ids: Vec<&str> = field(status, name)?.split_ascii_whitespace().collect();
        if ids.len() != 4 || ids.iter().any(|i| *i != id) {
            return Err(format!("{what}: {name} is {ids:?}, not four times {id}"));
        }
    }
    if !field(status, "Groups")?.is_empty() {
        return Err(format!("{what}: groups `{}`", field(status, "Groups")?));
    }
    for set in ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"] {
        let held = field(status, set)?;
        if u64::from_str_radix(held, 16) != Ok(0) {
            return Err(format!("{what}: {set} is {held}"));
        }
    }
    if field(status, "NoNewPrivs")? != "1" {
        return Err(format!("{what}: NoNewPrivs is not set"));
    }
    Ok(())
}

/// Suite 10: `qsvmm` runs as the unprivileged user and holds no
/// capability after its start, in any of its threads.
pub fn privilege(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, _) = cheap(run, 2)?;
    let deadline = Instant::now() + 3 * run.timeout;
    start(&mut guest, None)?;
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    guest_ready(&mut guest, deadline)?;
    let pid = pid(&mut guest)?;
    let channel = guest.channel(deadline)?;
    let status = channel.text(&format!("/proc/{pid}/status"))?;
    if field(&status, "Name")? != "qsvmm" {
        return Err(format!("process {pid} is `{}`", field(&status, "Name")?));
    }
    unprivileged(&status, "qsvmm")?;
    let tasks = channel.ls(&format!("/proc/{pid}/task"))?;
    let mut names = Vec::new();
    for task in &tasks {
        let status = channel.text(&format!("/proc/{pid}/task/{task}/status"))?;
        let name = field(&status, "Name")?.to_string();
        unprivileged(&status, &format!("thread {name}"))?;
        names.push(name);
    }
    // The log is the user's, for its generations; the socket is root's.
    if guest.said(&vmm_line("qsvmm: not started as root")) {
        return Err("qsvmm says it was not started as root".into());
    }
    end(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "qsvmm (process {pid}) and its {} threads ({}) run as user and group {}, with no \
         supplementary group, no capability in any set and NoNewPrivs set",
        tasks.len(),
        names.join(" "),
        qsvmm::PARTITION_ID
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DROPPED: &str = "Name:\tqsvmm\nUid:\t23000\t23000\t23000\t23000\n\
        Gid:\t23000\t23000\t23000\t23000\nGroups:\t\nCapInh:\t0000000000000000\n\
        CapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapBnd:\t0000000000000000\n\
        CapAmb:\t0000000000000000\nNoNewPrivs:\t1\n";

    #[test]
    fn a_dropped_process_passes_and_each_thing_kept_fails() {
        assert_eq!(unprivileged(DROPPED, "x"), Ok(()));
        assert_eq!(field(DROPPED, "Name"), Ok("qsvmm"));
        for (from, to) in [
            ("Uid:\t23000\t23000", "Uid:\t0\t23000"),
            (
                "Gid:\t23000\t23000\t23000\t23000",
                "Gid:\t23000\t23000\t0\t23000",
            ),
            ("Groups:\t", "Groups:\t0 "),
            ("CapBnd:\t0000000000000000", "CapBnd:\t000001ffffffffff"),
            ("CapEff:\t0000000000000000", "CapEff:\t0000000000000001"),
            ("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
        ] {
            assert!(DROPPED.contains(from));
            assert!(
                unprivileged(&DROPPED.replace(from, to), "x").is_err(),
                "{to}"
            );
        }
    }
}
