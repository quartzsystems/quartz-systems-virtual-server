// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition suites, on the test image. QEMU stands in for the
//! machine, the QS/VS test image is the host in it, and its partition
//! manager is asked for everything a partition does, through the test
//! hooks' channel: the same requests `docs/PARTD.md` documents. A
//! guest's disk is a further disk of the host, which `qsvmm` opens as
//! the block device it is.
//!
//! Three guests. The cheap one is QS/VS's own disk image: its UKI is a
//! bzImage of the same kernel family, and its PID 1 says what it does
//! on the serial port. The second is a QS/400 disk image, built by
//! QS/400's own orchestrator at the pinned commit and named by
//! [`QS400_IMAGE_ENV`]. The third does nothing but run
//! (`ipl::load::spinner`): a busy processor for the suites that
//! measure one.
//!
//! - [`manager`]: the partition manager from the harness.
//! - [`devices`]: the cheap guest: processors, virtio-scsi, privileges.
//! - [`qs400`]: QS/400: both sides, the IPL attributes, the ends, the
//!   side it asks for, its identity.
//! - [`pull`]: the host's power pulled under a guest that writes.
//! - [`damaged`]: load sources that cannot be started from.
//! - [`define`]: definitions and profiles, kept and validated.
//! - [`resources`]: activation's accounts, processors, memory.
//! - [`life`]: auto start, the ends, a killed VMM, a manager started
//!   again, isolation.

mod damaged;
mod define;
mod devices;
mod life;
pub mod manager;
mod pull;
mod qs400;
mod resources;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use model::profile::IplSource;
use model::{ConsoleMode, Keylock};
use partd::Calls as _;
use qsvshook::partition::{relayed, reported};
use qsvshook::proto::{command, word};
use refcode::RefCode;

use super::base::{Guest, Run, SuiteFn, booted, channel, posted};
use crate::qemu::image::sparse_copy;
use crate::qemu::session::Session;
use crate::qemu::{Drive, Launch};
pub use manager::Spec;

/// The suites: Phase 3's ten, in the order of its handoff, then
/// Phase 4's. Phase 3's eleventh, the fuzz targets, is the build
/// tool's: it compiles them with coverage.
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
    ("partition-recorded", qs400::recorded),
    ("partition-define", define::define),
    ("partition-three", resources::three),
    ("partition-minimum", resources::minimum),
    ("partition-dedicated", resources::dedicated),
    ("partition-capped", resources::capped),
    ("partition-uncapped", resources::uncapped),
    ("partition-core-sched", resources::core_scheduling),
    ("partition-huge-pages", resources::huge_pages),
    ("partition-auto-start", life::auto_start),
    ("partition-killed", life::killed),
    ("partition-manager-restart", life::manager_restart),
    ("partition-identity", qs400::identity),
    ("partition-isolation", life::isolation),
    ("partition-own-disks", devices::own_disks),
    ("partition-tools", life::tools),
];

/// The environment variable that names the QS/400 disk image.
pub const QS400_IMAGE_ENV: &str = "QSVS_QS400_IMAGE";

/// The partition's name.
pub const NAME: &str = "TESTLPAR";
/// Its ID: the first partition a host is given.
pub const ID: u32 = 1;
/// The guest's disk on the host: the first disk after the host's own.
pub const UNIT: &str = "/dev/vdb";
/// The host's memory: what is left of it after the host's reserve is
/// room for a partition of two gigabytes.
pub const HOST_MEMORY_MB: u32 = 5120;
/// The host's processors: one is the host's own.
pub const HOST_CPUS: u32 = 6;
/// A QS/400 partition's memory.
pub const QS400_MEMORY_MIB: u64 = 2048;
/// The cheap guest's memory.
pub const QSVS_MEMORY_MIB: u64 = 512;

/// The QS/400 disk image the environment names, when it is one: a
/// file whose table has an ESP and a LIC partition.
pub fn qs400_image() -> Result<PathBuf, String> {
    let how = "build it with QS/400's `cargo run -p build -- all` at the pinned commit \
               (docs/PHASE4.md)";
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

/// A profile for the partition: dedicated processors, that much
/// memory, and `ipl`, which is `side`, `keylock` and `console` as
/// words: `side b keylock manual console local`.
pub fn profile(processors: u32, memory_mib: u64, ipl: &str, backing: &str) -> Spec {
    let mut spec = Spec::new(processors, memory_mib, backing);
    let words: Vec<&str> = ipl.split_ascii_whitespace().collect();
    for pair in words.chunks(2) {
        match pair {
            ["side", "a"] => spec.side = IplSource::A,
            ["side", "b"] => spec.side = IplSource::B,
            ["side", "recorded"] => spec.side = IplSource::Recorded,
            ["keylock", "manual"] => spec.keylock = Some(Keylock::Manual),
            ["keylock", "normal"] => spec.keylock = Some(Keylock::Normal),
            ["console", "local"] => spec.console = ConsoleMode::Local,
            ["console", "serial"] => spec.console = ConsoleMode::Serial,
            other => panic!("a suite's profile says {other:?}"),
        }
    }
    spec
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
/// them [`UNIT`]. The host's own disk is a copy and no disk is a
/// snapshot: what a guest writes is kept from one launch to the next.
pub fn launch(run: &Run<'_>, units: &[PathBuf]) -> Result<Launch, String> {
    let host = run.file("host.img");
    if !host.is_file() {
        sparse_copy(run.disk, &host)?;
    }
    let extra: Vec<Drive> = units
        .iter()
        .enumerate()
        .map(|(i, path)| Drive {
            path: path.clone(),
            serial: format!("QSVSUNIT{i}"),
            place: None,
        })
        .collect();
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

/// Boot the host to "Base ready" with its channel, and wait for its
/// partition manager to answer.
pub fn host<'a>(run: &Run<'a>, launch: &Launch) -> Result<Guest<'a>, String> {
    let mut guest = booted(run, launch)?;
    let deadline = run.deadline();
    loop {
        match manager::partd(&mut guest)?.list() {
            Ok(_) => return Ok(guest),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!("the partition manager does not answer: {e}"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Boot the host, create the partition [`NAME`] and activate it with
/// `spec`.
pub fn hosted<'a>(run: &Run<'a>, launch: &Launch, spec: &Spec) -> Result<Guest<'a>, String> {
    let mut guest = host(run, launch)?;
    let id = manager::partition(&mut guest, NAME)?;
    if id != ID {
        return Err(format!("the first partition has the ID {id}"));
    }
    start(&mut guest, Some(spec))?;
    Ok(guest)
}

/// Ask the channel; an `error` answer is an error.
pub fn ask(guest: &mut Session, command: &str) -> Result<Vec<String>, String> {
    channel(guest)?.ask(command)
}

/// Wait for a code the host posts about the partition; what the
/// posting adds, after the partition's name.
pub fn expect_code(
    guest: &mut Session,
    code: &RefCode,
    deadline: Instant,
) -> Result<String, String> {
    guest.expect(&posted(code, ""), deadline)?;
    let rest = guest.rest_of_line(deadline)?;
    Ok(detail_of(&rest).to_string())
}

/// What a posting adds to its code's text, after the partition's name:
/// the line's rest is `: NAME, detail` or `: NAME`.
pub fn detail_of(rest: &str) -> &str {
    let rest = rest.trim_start_matches(':').trim();
    match rest.split_once(", ") {
        Some((name, detail)) if model::valid_name(name) => detail,
        _ if model::valid_name(rest) => "",
        _ => rest,
    }
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
    send_to(guest, ID, line)
}

/// Type a line at the console of the partition `id`.
pub fn send_to(guest: &mut Session, id: u32, line: &str) -> Result<(), String> {
    ask(guest, &format!("{} send {id} {line}", command::PARTITION)).map(|_| ())
}

/// Signal the partition's `qsvmm`: `TERM`, `KILL` or `USR1`.
pub fn signal(guest: &mut Session, signal: &str) -> Result<(), String> {
    signal_to(guest, ID, signal)
}

/// Signal the `qsvmm` of the partition `id`.
pub fn signal_to(guest: &mut Session, id: u32, signal: &str) -> Result<(), String> {
    ask(guest, &format!("{} kill {id} {signal}", command::PARTITION)).map(|_| ())
}

/// The process id of the `qsvmm` of the partition `id`.
pub fn pid_of(guest: &mut Session, id: u32) -> Result<u32, String> {
    let lines = ask(guest, &format!("{} pid {id}", command::PARTITION))?;
    lines
        .first()
        .and_then(|l| l.strip_prefix(word::OK))
        .and_then(|p| p.trim().parse().ok())
        .ok_or_else(|| format!("`partition pid {id}` answered {lines:?}"))
}

/// `qsvmm`'s process id.
pub fn pid(guest: &mut Session) -> Result<u32, String> {
    pid_of(guest, ID)
}

/// Wait for the partition to have no VMM; how the VMM ended: `exit
/// <code>` or `signal <number>`.
pub fn ended(guest: &mut Session, deadline: Instant) -> Result<String, String> {
    manager::ended(guest, ID, deadline).map(|status| status.last_end)
}

/// Activate the partition, with `spec` as its profile when one is
/// given and the profile it has otherwise.
pub fn start(guest: &mut Session, spec: Option<&Spec>) -> Result<(), String> {
    match spec {
        Some(spec) => manager::activate(guest, ID, spec)
            .map(|_| ())
            .map_err(|errors| manager::refused(&errors)),
        None => manager::said(manager::partd(guest)?.activate(ID, String::new(), None, None))
            .map(|_| ()),
    }
}

/// Power the host off in order and wait for the machine to be gone.
pub fn power_off(guest: &mut Guest<'_>, within: Duration) -> Result<(), String> {
    channel(guest)?.tell(command::POWER_OFF)?;
    let limit = Instant::now() + within;
    guest.expect(&posted(&refcode::POWER_OFF, ""), limit)?;
    guest.wait_exit(limit)?;
    Ok(())
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
    let log = qsvmm::log_dir(Path::new(release::DATA_DIR), ID).join(qsvmm::SERIAL_LOG);
    let pid = pid(guest).ok();
    let Ok(channel) = channel(guest) else {
        return "the channel does not answer".into();
    };
    match channel.cat(&log.to_string_lossy()) {
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
    let asked = pid.is_some() && signal(guest, "USR1").is_ok();
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
    // Only a line that has ended: a number may still be coming.
    let (line, _) = line.split_once('\n')?;
    let words: Vec<&str> = line.split_ascii_whitespace().collect();
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
    fn a_suite_s_profile_is_the_words_it_was_given() {
        let spec = profile(2, 2048, "side b keylock manual console local", UNIT);
        assert_eq!(spec.side, IplSource::B);
        assert_eq!(spec.keylock, Some(Keylock::Manual));
        assert_eq!(spec.console, ConsoleMode::Local);
        assert_eq!(spec.memory_mib, (2048, 2048));
        assert_eq!(spec.backing, [UNIT]);
        let plain = profile(1, 512, "", UNIT);
        assert_eq!(plain.side, IplSource::Recorded);
        assert_eq!((plain.keylock, plain.console), (None, ConsoleMode::Serial));
    }

    #[test]
    fn a_posting_s_detail_follows_the_partition_s_name() {
        assert_eq!(detail_of(": TESTLPAR, side A"), "side A");
        assert_eq!(detail_of(": TESTLPAR"), "");
        assert_eq!(detail_of(": TESTLPAR, exit 0"), "exit 0");
        assert_eq!(detail_of(""), "");
        // A detail that names no partition is left whole.
        assert_eq!(detail_of(": 3 defined, 1 running"), "3 defined, 1 running");
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
        // A line not yet ended is not read.
        assert_eq!(
            counts_in(
                "TESTLPAR! qsvmm: unit 0 reads 1 writes 2 flushes 3 refused 1",
                0
            ),
            None
        );
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
