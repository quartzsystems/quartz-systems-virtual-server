// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 3's suites: one partition on the test image. QEMU stands in
//! for the machine, the QS/VS test image is the host in it, and the
//! partition hook starts `qsvmm` for a profile the harness wrote. A
//! guest's disk is a further disk of the host, which `qsvmm` opens as
//! the block device it is.
//!
//! Two guests. The cheap one is QS/VS's own disk image: its UKI is a
//! bzImage of the same kernel family, and its PID 1 says what it does
//! on the serial port. The other is a QS/400 disk image, built by
//! QS/400's own orchestrator at the pinned commit and named by
//! [`QS400_IMAGE_ENV`]; the suites that IPL QS/400 need it.
//!
//! - [`devices`]: the cheap guest: processors, virtio-scsi, privileges.
//! - [`qs400`]: QS/400: both sides, the IPL attributes, the ends.
//! - [`pull`]: the host's power pulled under a guest that writes.
//! - [`damaged`]: load sources that cannot be started from.

mod damaged;
mod devices;
mod pull;
mod qs400;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use qsvshook::partition::{PROFILE, relayed, reported};
use qsvshook::proto::{command, word};
use refcode::RefCode;

use super::base::{Guest, Run, SuiteFn, booted, channel, posted};
use crate::qemu::image::{partition_volume, sparse_copy};
use crate::qemu::session::Session;
use crate::qemu::{Drive, Launch};

/// The suites, in the order of the handoff. Suite 11, the fuzz
/// targets, is the build tool's: it compiles them with coverage.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("partition-boot", devices::boot),
    ("partition-scsi", devices::scsi),
    ("partition-ipl-a", qs400::ipl_a),
    ("partition-ipl-b", qs400::ipl_b),
    ("partition-attributes", qs400::attributes),
    ("partition-kill", qs400::kill),
    ("partition-power-pull", pull::power_pull),
    ("partition-end", qs400::end),
    ("partition-damaged", damaged::damaged),
    ("partition-privilege", devices::privilege),
];

/// The environment variable that names the QS/400 disk image.
pub const QS400_IMAGE_ENV: &str = "QSVS_QS400_IMAGE";

/// The partition's name.
pub const NAME: &str = "TESTLPAR";
/// The guest's disk on the host: the first disk after the host's own.
pub const UNIT: &str = "/dev/vdb";
/// The host's memory: room for a partition of two gigabytes.
const HOST_MEMORY_MB: u32 = 4096;
/// The host's processors.
const HOST_CPUS: u32 = 4;
/// A QS/400 partition's memory.
pub const QS400_MEMORY_MIB: u32 = 2048;
/// The cheap guest's memory.
pub const QSVS_MEMORY_MIB: u32 = 512;

/// The QS/400 disk image the environment names, when it is one: a
/// file whose table has an ESP and a LIC partition.
pub fn qs400_image() -> Result<PathBuf, String> {
    let how = "build it with QS/400's `cargo run -p build -- all` at the pinned commit \
               (docs/PHASE3.md)";
    let path = std::env::var_os(QS400_IMAGE_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{QS400_IMAGE_ENV} is not set: {how}"))?;
    let mut file = std::fs::File::open(&path)
        .map_err(|e| format!("{QS400_IMAGE_ENV}={}: {e}", path.display()))?;
    let bytes = file.metadata().map_err(|e| e.to_string())?.len();
    let table = ipl::gpt::read(&mut file, bytes)
        .map_err(|e| format!("{QS400_IMAGE_ENV}={}: {e}", path.display()))?;
    let has = |ty| table.iter().any(|p| p.type_guid == ty);
    if has(ipl::ESP_TYPE) && has(ipl::LIC_TYPE) {
        Ok(path)
    } else {
        Err(format!(
            "{QS400_IMAGE_ENV}={} has no ESP and LIC partition: {how}",
            path.display()
        ))
    }
}

/// A profile for the partition: `ipl` is the lines of its `[ipl]` table.
pub fn profile(processors: u32, memory_mib: u32, ipl: &str, backing: &str) -> String {
    format!(
        "name = \"{NAME}\"\nprocessors = {processors}\nmemory_mib = {memory_mib}\n\n\
         [ipl]\n{ipl}\n[load_source]\nbacking = \"{backing}\"\n"
    )
}

/// A line of the partition's serial output, as the host relays it.
pub fn guest_line(text: &str) -> String {
    relayed(NAME, text)
}

/// A line of `qsvmm`'s own, as the host relays it.
pub fn vmm_line(text: &str) -> String {
    reported(NAME, text)
}

/// The launch of a host whose further disks are `units`, the first of
/// them [`UNIT`], and whose partition hook finds `profile`. The host's
/// own disk is a copy and no disk is a snapshot: what a guest writes
/// is kept from one launch to the next.
pub fn launch(run: &Run<'_>, units: &[PathBuf], profile: &str) -> Result<Launch, String> {
    let host = run.file("host.img");
    if !host.is_file() {
        sparse_copy(run.disk, &host)?;
    }
    let volume = run.file("partition.img");
    partition_volume(&volume, profile)?;
    let mut extra: Vec<Drive> = units
        .iter()
        .enumerate()
        .map(|(i, path)| Drive {
            path: path.clone(),
            serial: format!("QSVSUNIT{i}"),
            place: None,
        })
        .collect();
    extra.push(Drive {
        path: volume,
        serial: "QSVSPARTV".into(),
        place: None,
    });
    Ok(Launch {
        disk: Some(host),
        snapshot: false,
        memory_mb: HOST_MEMORY_MB,
        cpus: HOST_CPUS,
        prealloc: true,
        extra,
        ..run.launch()
    })
}

/// A fresh copy of the host's disk for this suite.
pub fn fresh_host(run: &Run<'_>) -> Result<(), String> {
    sparse_copy(run.disk, &run.file("host.img"))
}

/// A copy of `image` for this suite, as a guest's disk.
pub fn unit(run: &Run<'_>, image: &Path) -> Result<PathBuf, String> {
    let copy = run.file("unit.img");
    sparse_copy(image, &copy)?;
    Ok(copy)
}

/// Boot the host to "Base ready" with its channel.
pub fn host<'a>(run: &Run<'a>, launch: &Launch) -> Result<Guest<'a>, String> {
    booted(run, launch)
}

/// Ask the channel; an `error` answer is an error.
pub fn ask(guest: &mut Session, command: &str) -> Result<Vec<String>, String> {
    channel(guest)?.ask(command)
}

/// Wait for a code the host posts about the partition.
pub fn expect_code(
    guest: &mut Session,
    code: &RefCode,
    deadline: Instant,
) -> Result<String, String> {
    guest.expect(&posted(code, ""), deadline)?;
    guest.rest_of_line(deadline)
}

/// Wait for a line of the partition's serial output that has `text`.
pub fn expect_guest(guest: &mut Session, text: &str, deadline: Instant) -> Result<(), String> {
    guest.expect(&guest_line(text), deadline)
}

/// The codes of an activation, in order, to "Partition started".
pub fn expect_started(guest: &mut Session, deadline: Instant) -> Result<String, String> {
    for code in [
        &refcode::PARTITION_ACTIVATING,
        &refcode::PARTITION_DISKS_OPENED,
        &refcode::PARTITION_TABLE_READ,
        &refcode::PARTITION_IMAGE_READ,
        &refcode::PARTITION_LOADED,
        &refcode::PARTITION_MACHINE_BUILT,
    ] {
        guest.expect(&posted(code, ""), deadline)?;
    }
    expect_code(guest, &refcode::PARTITION_STARTED, deadline)
}

/// Type a line at the partition's console.
pub fn send(guest: &mut Session, line: &str) -> Result<(), String> {
    ask(guest, &format!("{} send {line}", command::PARTITION)).map(|_| ())
}

/// Signal `qsvmm`: `TERM` or `KILL`.
pub fn signal(guest: &mut Session, signal: &str) -> Result<(), String> {
    ask(guest, &format!("{} kill {signal}", command::PARTITION)).map(|_| ())
}

/// `qsvmm`'s process id.
pub fn pid(guest: &mut Session) -> Result<u32, String> {
    let lines = ask(guest, &format!("{} pid", command::PARTITION))?;
    lines
        .first()
        .and_then(|l| l.strip_prefix(word::OK))
        .and_then(|p| p.trim().parse().ok())
        .ok_or_else(|| format!("`partition pid` answered {lines:?}"))
}

/// Wait for the partition to have ended; how: `exit <code>` or
/// `signal <number>`.
pub fn ended(guest: &mut Session, deadline: Instant) -> Result<String, String> {
    loop {
        if let Ok(lines) = ask(guest, &format!("{} end", command::PARTITION))
            && let Some(how) = lines.first().and_then(|l| l.strip_prefix(word::OK))
        {
            return Ok(how.trim().to_string());
        }
        if Instant::now() >= deadline {
            return Err("the partition did not end in time".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Start the partition again, with `profile` when one is given.
pub fn start(guest: &mut Session, profile: Option<&str>) -> Result<(), String> {
    if let Some(profile) = profile {
        let put = format!("{} {PROFILE} {}", command::PUT, hex::encode(profile));
        ask(guest, &put)?;
    }
    ask(guest, &format!("{} start", command::PARTITION)).map(|_| ())
}

/// The host is still whole: its base ready and every service of it.
pub fn host_unaffected(guest: &mut Session) -> Result<(), String> {
    let status = channel(guest)?.status()?;
    super::base::all_ready(&status)
}

/// What can be learned of a partition that did not say what was
/// waited for: the end of its own serial log, as `qsvmm` wrote it,
/// and what its threads have used of the processor. For the reason of
/// a failure.
pub fn diagnosis(guest: &mut Session) -> String {
    let mut found = Vec::new();
    let log = format!("{}/log/partitions/{NAME}/serial.log", release::DATA_DIR);
    let Ok(channel) = channel(guest) else {
        return "the channel does not answer".into();
    };
    match channel.cat(&log) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let tail: Vec<&str> = text.lines().rev().take(6).collect();
            found.push(format!(
                "the partition's serial log has {} bytes, ending {:?}",
                bytes.len(),
                tail.into_iter().rev().collect::<Vec<_>>()
            ));
        }
        Err(e) => found.push(format!("its serial log: {e}")),
    }
    let pid = channel
        .ask(&format!("{} pid", command::PARTITION))
        .ok()
        .and_then(|l| {
            l.first()?
                .strip_prefix(word::OK)?
                .trim()
                .parse::<u32>()
                .ok()
        });
    match pid {
        Some(pid) => {
            let tasks = channel.ls(&format!("/proc/{pid}/task")).unwrap_or_default();
            let mut threads = Vec::new();
            for task in tasks {
                if let Ok(stat) = channel.text(&format!("/proc/{pid}/task/{task}/stat")) {
                    // The name is in parentheses; user and system time
                    // are the twelfth and thirteenth fields after it.
                    let name = stat.split(['(', ')']).nth(1).unwrap_or("?").to_string();
                    let rest: Vec<&str> = stat
                        .rsplit(')')
                        .next()
                        .unwrap_or("")
                        .split_ascii_whitespace()
                        .collect();
                    let ticks = |i: usize| rest.get(i).copied().unwrap_or("?");
                    threads.push(format!("{name} {}+{}", ticks(11), ticks(12)));
                }
            }
            found.push(format!(
                "qsvmm {pid} runs; its threads' user+system ticks: {}",
                threads.join(", ")
            ));
        }
        None => found.push("qsvmm is not running".into()),
    }
    // What the processors are doing: `qsvmm` says so on a signal, on
    // its standard error, which the host relays.
    let asked = pid.is_some()
        && channel
            .ask(&format!("{} kill USR1", command::PARTITION))
            .is_ok();
    if asked {
        std::thread::sleep(Duration::from_secs(3));
        guest.drain();
        let mark = vmm_line("qsvmm: vcpu");
        let states: Vec<&str> = guest
            .log
            .lines()
            .filter(|l| l.contains(&mark))
            .map(str::trim)
            .collect();
        found.push(format!("its processors: {states:?}"));
    }
    found.join("; ")
}

/// The counts `qsvmm` reports of a unit at its end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    /// Reads carried out.
    pub reads: u64,
    /// Writes carried out.
    pub writes: u64,
    /// Flushes of the host's device.
    pub flushes: u64,
    /// Requests refused.
    pub refused: u64,
}

/// The counts of `unit` in a line `qsvmm: unit N reads R writes W
/// flushes F refused X`.
pub fn counts_in(log: &str, unit: u32) -> Option<Counts> {
    let mark = vmm_line(&format!("qsvmm: unit {unit} "));
    let line = log.rsplit(&mark).next().filter(|_| log.contains(&mark))?;
    let words: Vec<&str> = line.lines().next()?.split_ascii_whitespace().collect();
    let value = |name: &str| -> Option<u64> {
        let at = words.iter().position(|w| *w == name)?;
        words.get(at + 1)?.parse().ok()
    };
    Some(Counts {
        reads: value("reads")?,
        writes: value("writes")?,
        flushes: value("flushes")?,
        refused: value("refused")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_one_qsvmm_reads() {
        let text = profile(2, 2048, "side = \"b\"\nconsole = \"serial\"", UNIT);
        let parsed = qsvmm::profile::parse(&text).unwrap();
        assert_eq!(parsed.name, NAME);
        assert_eq!((parsed.processors, parsed.memory_bytes), (2, 2048 << 20));
        assert_eq!(parsed.side, ipl::Side::B);
        assert_eq!(parsed.disks[0].backing, PathBuf::from(UNIT));
        assert!(qsvmm::profile::parse(&profile(1, 512, "", UNIT)).is_ok());
    }

    #[test]
    fn the_counts_are_read_from_the_last_report_of_a_unit() {
        let log = "TESTLPAR! qsvmm: unit 0 reads 1 writes 2 flushes 3 refused 4\r\n\
                   REF C2001005 Host running\r\n\
                   TESTLPAR! qsvmm: unit 0 reads 85 writes 28 flushes 31 refused 3\r\n\
                   TESTLPAR! qsvmm: TESTLPAR ended: signal 15\r\n";
        assert_eq!(
            counts_in(log, 0),
            Some(Counts {
                reads: 85,
                writes: 28,
                flushes: 31,
                refused: 3
            })
        );
        assert_eq!(counts_in(log, 1), None);
        assert_eq!(counts_in("", 0), None);
    }

    #[test]
    fn the_names_are_one_each() {
        let mut names: Vec<&str> = SUITES.iter().map(|s| s.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SUITES.len());
        assert_eq!(guest_line("x"), "TESTLPAR| x");
    }
}
