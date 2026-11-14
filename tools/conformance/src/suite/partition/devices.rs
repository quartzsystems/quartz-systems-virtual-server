// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The cheap guest: QS/VS's own disk image started in a partition. Its
//! UKI is a bzImage of the same kernel family and its PID 1 says on
//! the serial port how far it got.
//!
//! The image is a load-source disk of QS/VS, with the same partitions
//! and the same identifiers as the host's own. It is attached as it
//! is, a further disk of the host: the host takes the disk it started
//! from and the members its label names, and leaves this one alone
//! ([`own_disks`]). To read what the guest wrote, its data partition
//! is mounted on the host once the partition has ended.

use std::path::Path;
use std::time::{Duration, Instant};

use qsvshook::proto::{command, word};
use state::Inventory;

use super::{
    Counts, ID, NAME, QSVS_MEMORY_MIB, Spec, UNIT, ask, counts_in, ended, expect_code,
    expect_guest, fresh_host, host, host_unaffected, launch, manager, pid, profile, signal, start,
    unit, vmm_line,
};
use crate::qemu::image::{blank, data_partition};
use crate::qemu::session::Session;
use crate::suite::banner_prefix;
use crate::suite::base::{Guest, Run, posted};

/// The guest's data partition on the host: the third of its disk.
const GUEST_DATA: &str = "/dev/vdb3";
/// A blank disk, for the requests no guest sends.
const SCRATCH: &str = "/dev/vdc";
/// Where the guest's data partition is mounted.
const MOUNT: &str = "/run/guest";

fn cheap_profile(processors: u32) -> Spec {
    profile(processors, QSVS_MEMORY_MIB, "side a", UNIT)
}

/// Boot a host with the guest image and a blank disk as its further
/// disks, and define the partition. Nothing is activated yet.
fn cheap<'a>(run: &Run<'a>) -> Result<Guest<'a>, String> {
    // The release image: a test image's hooks want a second serial
    // port for their channel, which a partition has not.
    let image = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    fresh_host(run)?;
    let disk = unit(run, image)?;
    let scratch = run.file("scratch.img");
    blank(&scratch, data_partition(image)?.1)?;
    let mut guest = host(run, &launch(run, &[disk, scratch])?)?;
    let id = manager::partition(&mut guest, NAME)?;
    if id != ID {
        return Err(format!("the first partition has the ID {id}"));
    }
    Ok(guest)
}

/// A file of the guest's data file system, as the host reads it from
/// the guest's disk, and the names beside it.
fn guest_file(guest: &mut Session, path: &str) -> Result<(String, Vec<String>), String> {
    ask(guest, &format!("{} {GUEST_DATA} {MOUNT}", command::MOUNT))?;
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

/// The QS/VS UKI boots as a guest with 1, 2 and 4 processors and
/// prints its banner; the guest counts as many as it was given.
pub fn boot(run: &Run<'_>) -> Result<String, String> {
    let mut guest = cheap(run)?;
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
        let (text, _) = guest_file(&mut guest, "state/inventory.toml")?;
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
        "the QS/VS image, a disk of the host as it is, IPLed in a partition with {} \
         processors; the guest's inventory counts as many each time",
        seen.join(", ")
    ))
}

/// virtio-scsi. A guest reads and writes its LUN and what it wrote is
/// on the host's device; its flushes reach the host; requests beyond
/// the unit and requests that are none are refused.
pub fn scsi(run: &Run<'_>) -> Result<String, String> {
    let mut guest = cheap(run)?;
    let deadline = Instant::now() + 3 * run.timeout;
    let mut spec = cheap_profile(2);
    spec.serial = Some("TESTLPAR-0".into());
    start(&mut guest, Some(&spec))?;
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    guest_ready(&mut guest, deadline)?;
    std::thread::sleep(Duration::from_secs(2));
    end(&mut guest, deadline)?;
    // The whole line: its start may come before its end.
    guest.wait_for("the unit's counts", deadline, |log| {
        counts_in(log, 0).is_some()
    })?;
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
    // What the guest wrote, read by the host from the guest's disk.
    let (text, names) = guest_file(&mut guest, "state/inventory.toml")?;
    let inventory: Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
    let disk = inventory
        .disks
        .first()
        .ok_or("the guest's inventory has no disk")?;
    if !disk.model.contains("VIRTUAL DISK") || disk.serial != "TESTLPAR-0" {
        return Err(format!(
            "the guest's disk is a `{}` serial `{}`",
            disk.model, disk.serial
        ));
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
         the host, {refused} commands refused; the host reads the guest's state/ ({}) from the \
         guest's disk, which the guest has as a `{}` serial {}; probe of {SCRATCH}: {} checks, \
         {last}",
        names.join(" "),
        disk.model,
        disk.serial,
        seen.len()
    ))
}

/// A field of `/proc/<pid>/status`.
pub fn field<'t>(status: &'t str, name: &str) -> Result<&'t str, String> {
    status
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map(str::trim)
        .ok_or_else(|| format!("the status has no {name}"))
}

/// The identity and capabilities in one task's status: the user and
/// group `user` and nothing else.
pub fn unprivileged(status: &str, what: &str, user: u32) -> Result<(), String> {
    let id = user.to_string();
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

/// `qsvmm` runs as its partition's unprivileged user and holds no
/// capability after its start, in any of its threads.
pub fn privilege(run: &Run<'_>) -> Result<String, String> {
    let mut guest = cheap(run)?;
    let deadline = Instant::now() + 3 * run.timeout;
    start(&mut guest, Some(&cheap_profile(2)))?;
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    guest_ready(&mut guest, deadline)?;
    let user = model::definition::user_of(ID);
    let pid = pid(&mut guest)?;
    let channel = guest.channel(deadline)?;
    let status = channel.text(&format!("/proc/{pid}/status"))?;
    if field(&status, "Name")? != "qsvmm" {
        return Err(format!("process {pid} is `{}`", field(&status, "Name")?));
    }
    unprivileged(&status, "qsvmm", user)?;
    let tasks = channel.ls(&format!("/proc/{pid}/task"))?;
    let mut names = Vec::new();
    for task in &tasks {
        let status = channel.text(&format!("/proc/{pid}/task/{task}/status"))?;
        let name = field(&status, "Name")?.to_string();
        unprivileged(&status, &format!("thread {name}"), user)?;
        names.push(name);
    }
    if guest.said(&vmm_line("qsvmm: not started as root")) {
        return Err("qsvmm says it was not started as root".into());
    }
    end(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "qsvmm (process {pid}) and its {} threads ({}) run as user and group {user}, with no \
         supplementary group, no capability in any set and NoNewPrivs set",
        tasks.len(),
        names.join(" ")
    ))
}

/// The bytes of `file` before its data partition: the table, the ESP
/// and the record partition, where a host that took the disk for its
/// own would write.
fn head(file: &Path, bytes: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(file)
        .and_then(|f| f.take(bytes).read_to_end(&mut head))
        .map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(head)
}

/// The host takes only its own load-source disks: a guest's disk that
/// looks like a QS/VS load source is attached as it is and left
/// alone, and still IPLs in a partition.
pub fn own_disks(run: &Run<'_>) -> Result<String, String> {
    let image = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    let (data_at, _) = data_partition(image)?;
    let before = head(image, data_at)?;
    let mut guest = cheap(run)?;
    let disk = run.file("unit.img");
    // The host has one load-source unit, the disk it IPLed from.
    let storage = guest.channel(run.deadline())?.storage()?;
    if storage.units.len() != 1 || storage.units[0].disk != "vda" {
        return Err(format!(
            "the host's load source is {:?}",
            storage
                .units
                .iter()
                .map(|u| u.disk.as_str())
                .collect::<Vec<_>>()
        ));
    }
    // An IPL of the host, its slot confirmed, a power off: the host
    // wrote its record, and not to the guest's disk.
    let status = guest.channel(run.deadline())?.status()?;
    super::super::base::all_ready(&status)?;
    guest
        .channel(run.deadline())?
        .tell(command::POWER_OFF)
        .map_err(|e| format!("power off: {e}"))?;
    guest.expect(&posted(&refcode::POWER_OFF, ""), run.deadline())?;
    guest.wait_exit(run.deadline())?;
    drop(guest);
    if head(&disk, data_at)? != before {
        return Err(
            "the host wrote to the table, the ESP or the record partition of a guest's disk".into(),
        );
    }

    // The disk is a partition's: it IPLs from it.
    let mut guest = host(run, &launch(run, &[disk.clone(), run.file("scratch.img")])?)?;
    let deadline = Instant::now() + 3 * run.timeout;
    start(&mut guest, Some(&cheap_profile(1)))?;
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    guest_ready(&mut guest, deadline)?;
    end(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "a disk with QS/VS's own three partitions and identifiers, attached as it is: the \
         host's load source is vda alone, and after an IPL and a power off of the host the \
         first {data_at} bytes of the guest's disk (table, ESP, record partition) are as they \
         were; the disk then IPLs in a partition"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DROPPED: &str = "Name:\tqsvmm\nUid:\t23001\t23001\t23001\t23001\n\
        Gid:\t23001\t23001\t23001\t23001\nGroups:\t\nCapInh:\t0000000000000000\n\
        CapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapBnd:\t0000000000000000\n\
        CapAmb:\t0000000000000000\nNoNewPrivs:\t1\n";

    #[test]
    fn a_dropped_process_passes_and_each_thing_kept_fails() {
        assert_eq!(unprivileged(DROPPED, "x", 23_001), Ok(()));
        assert!(unprivileged(DROPPED, "x", 23_002).is_err());
        assert_eq!(field(DROPPED, "Name"), Ok("qsvmm"));
        for (from, to) in [
            ("Uid:\t23001\t23001", "Uid:\t0\t23001"),
            (
                "Gid:\t23001\t23001\t23001\t23001",
                "Gid:\t23001\t23001\t0\t23001",
            ),
            ("Groups:\t", "Groups:\t0 "),
            ("CapBnd:\t0000000000000000", "CapBnd:\t000001ffffffffff"),
            ("CapEff:\t0000000000000000", "CapEff:\t0000000000000001"),
            ("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
        ] {
            assert!(DROPPED.contains(from));
            assert!(
                unprivileged(&DROPPED.replace(from, to), "x", 23_001).is_err(),
                "{to}"
            );
        }
    }
}
