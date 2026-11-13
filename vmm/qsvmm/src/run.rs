// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition from its configuration to its end, in order:
//!
//! 1. The configuration is read: from standard input, where the
//!    partition manager writes it, or from a file a developer names.
//! 2. As root: KVM is opened, the log and the console socket are
//!    made, the disk units and the random source are opened, the
//!    guest's memory is mapped, the IPL loader reads the load source
//!    and loads the side's kernel, the devices and processors are
//!    built.
//! 3. The process becomes the partition's own unprivileged user.
//!    Nothing is opened after this but generations of the serial log,
//!    in a directory that is that user's and no other partition's.
//! 4. The threads are started and the processors run until one of
//!    them, a device or a signal ends the partition.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use arch::layout::Ram;
use devices::platform::{Request, Requests};
use devices::virtio::scsi::backing::HostStore;
use devices::virtio::scsi::lun::Lun;
use ipl::{Fault, Step};
use refcode::RefCode;
use vmm_sys_util::signal::Killable;

use crate::config::{self, Partition};
use crate::end::End;
use crate::machine::{self, Hypervisor, vcpu};
use crate::serial::{self, Clients, console, log::Log};
use crate::{post, sys};

/// Where the partition's files go.
#[derive(Clone, Debug)]
pub struct Options {
    /// The configuration's file; standard input when there is none.
    pub config: Option<PathBuf>,
    /// The data directory: the serial log is under it.
    pub data_dir: PathBuf,
    /// The run directory: the console socket is under it.
    pub run_dir: PathBuf,
}

impl Options {
    /// `<data>/log/partitions/<id>`.
    pub fn log_dir(&self, id: u32) -> PathBuf {
        crate::log_dir(&self.data_dir, id)
    }

    /// `<run>/partitions/<id>`.
    pub fn socket_dir(&self, id: u32) -> PathBuf {
        crate::socket_dir(&self.run_dir, id)
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
                "side {}{} keylock {} console {} sequence {}",
                ipl::side_letter(recorded.side),
                if recorded.once { " once" } else { "" },
                recorded.keylock.attribute(),
                ipl::lic::console_name(recorded.console),
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
    luns: Vec<Arc<Vec<Lun>>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The units of every adapter, opened.
fn open_units(partition: &Partition) -> Result<Vec<Vec<Lun>>, Stop> {
    let mut adapters = Vec::with_capacity(partition.adapters.len());
    for adapter in &partition.adapters {
        let mut luns = Vec::with_capacity(adapter.units.len());
        for unit in &adapter.units {
            let unreadable = |why: String| Stop {
                code: &refcode::PARTITION_DISK_UNREADABLE,
                detail: format!(
                    "slot {} unit {} {}: {why}",
                    adapter.slot,
                    unit.lun,
                    unit.backing.display()
                ),
            };
            let store = HostStore::open(&unit.backing).map_err(|e| unreadable(e.to_string()))?;
            let lun = Lun::new(Box::new(store), &unit.serial)
                .ok_or_else(|| unreadable("not a disk unit".into()))?;
            luns.push(lun);
        }
        adapters.push(luns);
    }
    Ok(adapters)
}

/// Make `dir` the partition's own: its user's, and nobody else's to
/// look into.
fn own(dir: &Path, user: u32) -> io::Result<()> {
    chown(dir, Some(user), Some(user))?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

/// The serial log, in a directory the partition's user rotates it in.
fn open_log(options: &Options, partition: &Partition, root: bool) -> Result<Log, Stop> {
    let log_dir = options.log_dir(partition.config.id);
    let log = Log::open(&log_dir, serial::log::LIMIT_BYTES).map_err(not_built("the serial log"))?;
    if root {
        let user = partition.user();
        own(&log_dir, user)
            .and_then(|()| {
                // Every generation left by an earlier activation too.
                for entry in fs::read_dir(&log_dir)?.flatten() {
                    chown(entry.path(), Some(user), Some(user))?;
                }
                Ok(())
            })
            .map_err(not_built("the serial log"))?;
    }
    Ok(log)
}

/// The console socket, the partition's user's as its directory is.
fn open_console(
    options: &Options,
    partition: &Partition,
    root: bool,
) -> Result<std::os::unix::net::UnixListener, Stop> {
    let dir = options.socket_dir(partition.config.id);
    let listener = console::bind(&dir).map_err(not_built("the console socket"))?;
    if root {
        let user = partition.user();
        own(&dir, user)
            .and_then(|()| chown(dir.join(console::FILE), Some(user), Some(user)))
            .map_err(not_built("the console socket"))?;
    }
    Ok(listener)
}

/// The IPL: the load source read through the page cache, and what the
/// loader wrote made durable before the guest's first direct read.
fn load(partition: &Partition, memory: &devices::Memory, ram: &Ram) -> Result<ipl::Ipl, Stop> {
    let load_source = partition.load_source();
    let unreadable = |e: io::Error| Stop {
        code: &refcode::PARTITION_DISK_UNREADABLE,
        detail: format!("the load source {}: {e}", load_source.display()),
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
        partition.side,
        partition.settings,
        now(),
        memory,
        ram,
        &mut post_step,
    )
    .map_err(|fault| fault_stop(&fault))?;
    disk.sync_all().map_err(unreadable)?;
    Ok(loaded)
}

/// The guest's memory with everything the kernel starts from in it:
/// the kernel itself, the ACPI tables, the reset vector, the SMBIOS
/// tables.
fn load_memory(
    partition: &Partition,
    hypervisor: &Hypervisor,
    ram: &Ram,
) -> Result<(Arc<devices::Memory>, ipl::Ipl), Stop> {
    let config = &partition.config;
    // Huge pages that are not there are not a fault of the machine:
    // the partition was promised memory the host does not have.
    let memory = machine::memory(&hypervisor.vm, ram, config.huge_pages).map_err(|e| Stop {
        code: if config.huge_pages {
            &refcode::PARTITION_NO_MEMORY
        } else {
            &refcode::PARTITION_NOT_BUILT
        },
        detail: format!("{} MiB: {e}", config.memory_mib),
    })?;
    let loaded = load(partition, &memory, ram)?;
    machine::write_acpi(&memory, config.vcpus).map_err(not_built("ACPI"))?;
    let identity = arch::boot::smbios::Identity {
        serial: &config.serial,
        release: &release::RELEASE.to_string(),
    };
    arch::boot::smbios::write(&memory, identity)
        .map_err(|e| not_built("SMBIOS")(io::Error::other(e)))?;
    Ok((memory, loaded))
}

/// Say the machine is built, and give the process its core scheduling
/// cookie: before any thread exists, so every thread shares it, and no
/// other partition's thread runs beside one of these on a core.
fn machine_built(partition: &Partition) -> Result<(), Stop> {
    let config = &partition.config;
    post(
        &refcode::PARTITION_MACHINE_BUILT,
        &format!(
            "{} processor(s) of {} thread(s), {} MiB{}, {} disk unit(s)",
            config.vcpus / config.threads_per_core,
            config.threads_per_core,
            config.memory_mib,
            if config.huge_pages {
                " in huge pages"
            } else {
                ""
            },
            partition.units()
        ),
    );
    match sys::own_core_cookie() {
        Ok(true) => eprintln!("qsvmm: core scheduling: a cookie of its own"),
        Ok(false) => eprintln!("qsvmm: core scheduling: the host has none to do"),
        Err(e) => return Err(not_built("core scheduling")(e)),
    }
    Ok(())
}

/// The steps as root, the drop, and the start.
fn activate(options: &Options, partition: &Partition) -> Result<Running, Stop> {
    let root = sys::is_root();
    let config = &partition.config;
    let ram = Ram::for_bytes(partition.memory_bytes).map_err(|e| Stop {
        code: &refcode::PARTITION_PROFILE_INVALID,
        detail: e.to_string(),
    })?;
    let hypervisor = Hypervisor::open().map_err(not_built("KVM"))?;
    if config.vcpus as usize > hypervisor.max_vcpus() {
        return stop(
            &refcode::PARTITION_NOT_BUILT,
            format!("KVM gives {} processors at most", hypervisor.max_vcpus()),
        );
    }

    let log = open_log(options, partition, root)?;
    let listener = open_console(options, partition, root)?;

    let adapters = open_units(partition)?;
    post(
        &refcode::PARTITION_DISKS_OPENED,
        &format!(
            "{} unit(s), load source {} blocks",
            partition.units(),
            adapters[0][0].blocks()
        ),
    );
    let entropy = File::open("/dev/urandom").map_err(not_built("/dev/urandom"))?;
    let (memory, loaded) = load_memory(partition, &hypervisor, &ram)?;
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
        adapters,
        entropy,
    )
    .map_err(not_built("devices"))?;
    let mut vcpus = Vec::with_capacity(config.vcpus as usize);
    for index in 0..config.vcpus {
        let created = vcpu::create(
            &hypervisor.kvm,
            &hypervisor.vm,
            index,
            config.vcpus,
            config.threads_per_core,
            loaded.entry,
        )
        .map_err(not_built("processors"))?;
        vcpus.push(created);
    }
    machine_built(partition)?;

    // From here on nothing needs root, and no thread exists yet.
    sys::block_termination().map_err(not_built("signals"))?;
    if root {
        sys::drop_privileges(partition.user()).map_err(not_built("dropping privileges"))?;
    } else {
        eprintln!("qsvmm: not started as root: running as the user it was started by");
    }

    let clients = Clients::default();
    serial::spawn(queue, log, clients.clone()).map_err(not_built("threads"))?;
    console::spawn(listener, built.serial.clone(), clients).map_err(not_built("threads"))?;
    vcpu::install_kick().map_err(not_built("signals"))?;
    let processors = start_processors(vcpus, &built, &ends, &memory, &config.pin)
        .map_err(not_built("threads"))?;
    spawn_signal_watch(ends.clone(), processors).map_err(not_built("threads"))?;
    post(
        &refcode::PARTITION_STARTED,
        &format!("side {}", ipl::side_letter(loaded.side)),
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

/// A processor's thread and the flag that has it report its state.
type Processor = (JoinHandle<()>, Arc<AtomicBool>);

/// Start a thread for each processor.
fn start_processors(
    vcpus: Vec<kvm_ioctls::VcpuFd>,
    built: &machine::Devices,
    ends: &Sender<(End, String)>,
    memory: &Arc<devices::Memory>,
    pin: &[u32],
) -> io::Result<Vec<Processor>> {
    let mut processors = Vec::with_capacity(vcpus.len());
    for (index, created) in (0u32..).zip(vcpus) {
        let report = Arc::new(AtomicBool::new(false));
        let wiring = vcpu::Wiring {
            io: built.io.clone(),
            pci: built.pci.clone(),
            ends: ends.clone(),
            report: report.clone(),
            memory: memory.clone(),
            pin: pin.get(index as usize).copied(),
        };
        processors.push((vcpu::spawn(created, index, wiring)?, report));
    }
    Ok(processors)
}

/// Have every processor say what state it is in. A thread is brought
/// out of the guest by a signal, which it may miss on its way in: it
/// is sent again until the thread has answered.
fn report_processors(processors: &[Processor]) {
    for (_, report) in processors {
        report.store(true, Ordering::Relaxed);
    }
    for _ in 0..40 {
        let waiting: Vec<&Processor> = processors
            .iter()
            .filter(|(_, report)| report.load(Ordering::Relaxed))
            .collect();
        if waiting.is_empty() {
            return;
        }
        for (thread, _) in waiting {
            let _ = thread.kill(vcpu::kick_signal());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// The thread that takes the signals: one asks for a report of the
/// processors, any other ends the partition.
fn spawn_signal_watch(ends: Sender<(End, String)>, processors: Vec<Processor>) -> io::Result<()> {
    thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let why = loop {
                match sys::wait_for_termination() {
                    Ok(sys::REPORT) => report_processors(&processors),
                    Ok(signal) => break format!("signal {signal}"),
                    Err(e) => break format!("waiting for a signal: {e}"),
                }
            };
            let _ = ends.send((End::Ended, why));
        })?;
    Ok(())
}

/// The configuration's text: the file's, or what standard input has
/// to its end. Bounded either way.
fn read_config(options: &Options) -> io::Result<String> {
    let mut text = String::new();
    match &options.config {
        Some(path) => File::open(path)?
            .take(config::MAX_BYTES)
            .read_to_string(&mut text)?,
        None => io::stdin()
            .lock()
            .take(config::MAX_BYTES)
            .read_to_string(&mut text)?,
    };
    Ok(text)
}

/// Run the partition of `options` to its end.
pub fn run(options: &Options) -> End {
    let text = match read_config(options) {
        Ok(text) => text,
        Err(e) => {
            post(
                &refcode::PARTITION_PROFILE_INVALID,
                &format!("the configuration: {e}"),
            );
            return End::NotIpled;
        }
    };
    let partition = match config::parse(&text) {
        Ok(partition) => partition,
        Err(e) => {
            post(&refcode::PARTITION_PROFILE_INVALID, &e.0);
            return End::NotIpled;
        }
    };
    let name = partition.config.name.clone();
    crate::name_partition(&name);
    post(&refcode::PARTITION_ACTIVATING, "");
    let running = match activate(options, &partition) {
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
    let mut unit = 0;
    for luns in &running.luns {
        for lun in luns.iter() {
            let c = lun.counts();
            eprintln!(
                "qsvmm: unit {unit} reads {} writes {} flushes {} refused {}",
                c.reads, c.writes, c.flushes, c.refused
            );
            unit += 1;
        }
    }
    eprintln!("qsvmm: {name} ended: {why}");
    end
}
