// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition from its profile to its end, in order:
//!
//! 1. The profile is read.
//! 2. As root: KVM is opened, the log and the console socket are
//!    made, the disk units and the random source are opened, the
//!    guest's memory is mapped, the IPL loader reads the load source
//!    and loads the side's kernel, the devices and processors are
//!    built.
//! 3. The process becomes the unprivileged user. Nothing is opened
//!    after this but generations of the serial log, in a directory
//!    that is that user's.
//! 4. The threads are started and the processors run until one of
//!    them, a device or a signal ends the partition.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::os::unix::fs::chown;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arch::layout::Ram;
use devices::platform::{Request, Requests};
use devices::virtio::scsi::backing::HostStore;
use devices::virtio::scsi::lun::Lun;
use ipl::{Fault, Step};
use refcode::RefCode;

use crate::end::End;
use crate::machine::{self, Hypervisor, vcpu};
use crate::profile::{self, Profile};
use crate::serial::{self, Clients, console, log::Log};
use crate::{PARTITION_ID, post, sys};

/// Where the partition's files go.
#[derive(Clone, Debug)]
pub struct Options {
    /// The profile.
    pub profile: PathBuf,
    /// The data directory: the serial log is under it.
    pub data_dir: PathBuf,
    /// The run directory: the console socket is under it.
    pub run_dir: PathBuf,
}

impl Options {
    /// `<data>/log/partitions/<name>`.
    pub fn log_dir(&self, name: &str) -> PathBuf {
        self.data_dir.join("log").join("partitions").join(name)
    }

    /// `<run>/partitions/<name>`.
    pub fn socket_dir(&self, name: &str) -> PathBuf {
        self.run_dir.join("partitions").join(name)
    }
}

/// Why the partition did not start.
struct Stop {
    code: &'static RefCode,
    detail: String,
}

fn stop<T>(code: &'static RefCode, detail: impl Into<String>) -> Result<T, Stop> {
    Err(Stop {
        code,
        detail: detail.into(),
    })
}

fn not_built(what: &str) -> impl Fn(io::Error) -> Stop + '_ {
    move |e| Stop {
        code: &refcode::PARTITION_NOT_BUILT,
        detail: format!("{what}: {e}"),
    }
}

fn post_step(step: Step) {
    match step {
        Step::TableRead => post(&refcode::PARTITION_TABLE_READ, ""),
        Step::RecordRead(recorded) => post(
            &refcode::PARTITION_RECORD_READ,
            &format!(
                "side {} keylock {} console {} sequence {}",
                ipl::side_letter(recorded.side),
                recorded.keylock.attribute(),
                recorded.console.name(),
                recorded.sequence
            ),
        ),
        Step::RecordWritten => post(&refcode::PARTITION_RECORD_WRITTEN, ""),
        Step::NoRecord => post(&refcode::PARTITION_NO_RECORD, ""),
        Step::ImageRead { side, bytes } => post(
            &refcode::PARTITION_IMAGE_READ,
            &format!("side {} {bytes} bytes", ipl::side_letter(side)),
        ),
        Step::Loaded => post(&refcode::PARTITION_LOADED, ""),
    }
}

fn fault_stop(fault: &Fault) -> Stop {
    let code = match fault {
        Fault::Io => &refcode::PARTITION_DISK_UNREADABLE,
        Fault::NoTable(_) => &refcode::PARTITION_NO_TABLE,
        Fault::NoEsp => &refcode::PARTITION_NO_ESP,
        Fault::BadEsp(_) => &refcode::PARTITION_BAD_ESP,
        Fault::NoImage(_) => &refcode::PARTITION_NO_IMAGE,
        Fault::BadImage(..) => &refcode::PARTITION_BAD_IMAGE,
        Fault::NoLic => &refcode::PARTITION_NO_LIC,
        Fault::BadRecord(_) => &refcode::PARTITION_BAD_RECORD,
        Fault::NotKernel(ipl::load::LoadError::NotKernel(_)) => &refcode::PARTITION_NOT_KERNEL,
        Fault::NotKernel(_) => &refcode::PARTITION_NOT_BUILT,
    };
    let detail = match fault {
        Fault::Io => "the load source".to_string(),
        Fault::NoEsp | Fault::NoLic => String::new(),
        Fault::NoImage(side) => format!("side {}", ipl::side_letter(*side)),
        other => other.to_string(),
    };
    Stop { code, detail }
}

/// A partition that is running.
struct Running {
    ends: mpsc::Receiver<(End, String)>,
    tap: serial::Tap,
    luns: Arc<Vec<Lun>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn open_units(profile: &Profile) -> Result<Vec<Lun>, Stop> {
    let mut luns = Vec::with_capacity(profile.disks.len());
    for (unit, disk) in profile.disks.iter().enumerate() {
        let unreadable = |why: String| Stop {
            code: &refcode::PARTITION_DISK_UNREADABLE,
            detail: format!("unit {unit} {}: {why}", disk.backing.display()),
        };
        let store = HostStore::open(&disk.backing).map_err(|e| unreadable(e.to_string()))?;
        let lun = Lun::new(Box::new(store), &disk.serial)
            .ok_or_else(|| unreadable("not a disk unit".into()))?;
        luns.push(lun);
    }
    Ok(luns)
}

/// The serial log, in a directory the unprivileged user rotates it in.
fn open_log(options: &Options, profile: &Profile, root: bool) -> Result<Log, Stop> {
    let log_dir = options.log_dir(&profile.name);
    let log = Log::open(&log_dir, serial::log::LIMIT_BYTES).map_err(not_built("the serial log"))?;
    if root {
        let own = |path: &Path| chown(path, Some(PARTITION_ID), Some(PARTITION_ID));
        own(&log_dir)
            .and_then(|()| own(&log_dir.join(serial::log::FILE)))
            .map_err(not_built("the serial log"))?;
    }
    Ok(log)
}

/// The IPL: the load source read through the page cache, and what the
/// loader wrote made durable before the guest's first direct read.
fn load(profile: &Profile, memory: &devices::Memory, ram: &Ram) -> Result<ipl::Ipl, Stop> {
    let load_source = &profile.disks[0].backing;
    let unreadable = |e: io::Error| Stop {
        code: &refcode::PARTITION_DISK_UNREADABLE,
        detail: format!("unit 0 {}: {e}", load_source.display()),
    };
    let mut disk = OpenOptions::new()
        .read(true)
        .write(true)
        .open(load_source)
        .map_err(unreadable)?;
    let disk_bytes = disk.seek(SeekFrom::End(0)).map_err(unreadable)?;
    let loaded = ipl::ipl(
        &mut disk,
        disk_bytes,
        profile.side,
        profile.settings,
        now(),
        memory,
        ram,
        &mut post_step,
    )
    .map_err(|fault| fault_stop(&fault))?;
    disk.sync_all().map_err(unreadable)?;
    Ok(loaded)
}

/// The steps as root, the drop, and the start.
fn activate(options: &Options, profile: &Profile) -> Result<Running, Stop> {
    let root = sys::is_root();
    let ram = Ram::for_bytes(profile.memory_bytes).map_err(|e| Stop {
        code: &refcode::PARTITION_PROFILE_INVALID,
        detail: e.to_string(),
    })?;
    let hypervisor = Hypervisor::open().map_err(not_built("KVM"))?;
    if profile.processors as usize > hypervisor.max_vcpus() {
        return stop(
            &refcode::PARTITION_NOT_BUILT,
            format!("KVM gives {} processors at most", hypervisor.max_vcpus()),
        );
    }

    let log = open_log(options, profile, root)?;
    let listener = console::bind(&options.socket_dir(&profile.name))
        .map_err(not_built("the console socket"))?;

    let luns = open_units(profile)?;
    post(
        &refcode::PARTITION_DISKS_OPENED,
        &format!(
            "{} unit(s), load source {} blocks",
            luns.len(),
            luns[0].blocks()
        ),
    );
    let entropy = File::open("/dev/urandom").map_err(not_built("/dev/urandom"))?;
    let memory = machine::memory(&hypervisor.vm, &ram).map_err(not_built("memory"))?;
    let loaded = load(profile, &memory, &ram)?;

    machine::write_acpi(&memory, profile.processors).map_err(not_built("ACPI"))?;
    let (ends, ended) = mpsc::channel::<(End, String)>();
    let (tap, queue) = serial::queue();
    let requests: Requests = {
        let ends = ends.clone();
        Box::new(move |request| {
            let end = match request {
                Request::PowerOff => End::PoweredOff,
                Request::Restart => End::Restart,
            };
            let _ = ends.send((end, "the guest asked".into()));
        })
    };
    let built = machine::devices(
        &hypervisor.vm,
        &memory,
        Box::new(tap.clone()),
        requests,
        luns,
        entropy,
    )
    .map_err(not_built("devices"))?;
    let mut vcpus = Vec::with_capacity(profile.processors as usize);
    for index in 0..profile.processors {
        let created = vcpu::create(
            &hypervisor.kvm,
            &hypervisor.vm,
            index,
            profile.processors,
            loaded.entry,
        )
        .map_err(not_built("processors"))?;
        vcpus.push(created);
    }
    post(
        &refcode::PARTITION_MACHINE_BUILT,
        &format!(
            "{} processor(s), {} MiB, {} disk unit(s)",
            profile.processors,
            profile.memory_bytes >> 20,
            built.luns.len()
        ),
    );

    // From here on nothing needs root, and no thread exists yet.
    sys::block_termination().map_err(not_built("signals"))?;
    if root {
        sys::drop_privileges(PARTITION_ID).map_err(not_built("dropping privileges"))?;
    } else {
        eprintln!("qsvmm: not started as root: running as the user it was started by");
    }

    let clients = Clients::default();
    serial::spawn(queue, log, clients.clone()).map_err(not_built("threads"))?;
    console::spawn(listener, built.serial.clone(), clients).map_err(not_built("threads"))?;
    spawn_signal_watch(ends.clone()).map_err(not_built("threads"))?;
    for (index, created) in (0u32..).zip(vcpus) {
        let wiring = vcpu::Wiring {
            io: built.io.clone(),
            pci: built.pci.clone(),
            ends: ends.clone(),
        };
        vcpu::spawn(created, index, wiring).map_err(not_built("threads"))?;
    }
    post(
        &refcode::PARTITION_STARTED,
        &format!("{} side {}", profile.name, ipl::side_letter(loaded.side)),
    );
    // The VM and the memory live as long as the process does.
    std::mem::forget(hypervisor);
    std::mem::forget(memory);
    Ok(Running {
        ends: ended,
        tap,
        luns: built.luns,
    })
}

fn spawn_signal_watch(ends: Sender<(End, String)>) -> io::Result<()> {
    thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let why = match sys::wait_for_termination() {
                Ok(signal) => format!("signal {signal}"),
                Err(e) => format!("waiting for a signal: {e}"),
            };
            let _ = ends.send((End::Ended, why));
        })?;
    Ok(())
}

/// Run the partition of `options` to its end.
pub fn run(options: &Options) -> End {
    let text = match fs::read_to_string(&options.profile) {
        Ok(text) => text,
        Err(e) => {
            post(
                &refcode::PARTITION_PROFILE_INVALID,
                &format!("{}: {e}", options.profile.display()),
            );
            return End::NotIpled;
        }
    };
    let profile = match profile::parse(&text) {
        Ok(profile) => profile,
        Err(e) => {
            post(&refcode::PARTITION_PROFILE_INVALID, &e.0);
            return End::NotIpled;
        }
    };
    post(&refcode::PARTITION_ACTIVATING, &profile.name);
    let running = match activate(options, &profile) {
        Ok(running) => running,
        Err(stopped) => {
            post(stopped.code, &stopped.detail);
            return End::NotIpled;
        }
    };
    let (end, why) = running
        .ends
        .recv()
        .unwrap_or((End::VmmError, "every thread ended".into()));
    // What the guest said last is in the log before the process goes.
    running.tap.drain(Duration::from_secs(2));
    for (unit, lun) in running.luns.iter().enumerate() {
        let c = lun.counts();
        eprintln!(
            "qsvmm: unit {unit} reads {} writes {} flushes {} refused {}",
            c.reads, c.writes, c.flushes, c.refused
        );
    }
    eprintln!("qsvmm: {} ended: {why}", profile.name);
    end
}
