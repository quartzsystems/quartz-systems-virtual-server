// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service. In order:
//!
//! 1. The host's cores and memory are taken, the settings read, and
//!    what the kernel set aside as huge pages counted.
//! 2. The cgroup tree's base is made.
//! 3. The partitions that run already are taken up: the service was
//!    started again and its partitions were not ended for that.
//! 4. The socket is bound and the service says it is ready.
//! 5. When the host's base is ready, the partitions flagged to start
//!    with it are activated, once an IPL.
//!
//! - [`machine`]: what the manager asks of the host.
//! - [`watch`]: a VMM's report and its end.
//! - [`sys`]: starting a process in its control group.

pub mod machine;
mod sys;
pub mod watch;

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use ipc::frame::{read_frame_max, write_frame};
use resource::settings::HUGE_PAGE_MIB;
use resource::{Host, Settings, Thread, Topology};
use supervise::control::Calls as _;

use crate::manager::{Ended, Manager, Record};
use crate::proto::{self, MAX_FRAME, Partitions as _, RUN, Request};
use crate::store::Store;
use machine::{Machine, Paths, Watch};
use watch::Shared;

/// The file that says the partitions flagged to start with the host
/// were started at this IPL.
const AUTO_STARTED: &str = "autostart";
/// How long a client is given to send its request.
const REQUEST_WAIT: Duration = Duration::from_secs(2);
/// How long auto start waits for the host's base to be ready.
const BASE_WAIT: Duration = Duration::from_secs(300);

/// What the service is given by the appliance it runs as.
#[derive(Clone, Debug)]
pub struct Service {
    /// The state directory.
    pub state_dir: PathBuf,
    /// The run directory.
    pub run_dir: PathBuf,
    /// PID 1's control socket.
    pub control_socket: PathBuf,
    /// The cgroup2 mount.
    pub cgroups: PathBuf,
    /// The kernel's huge page count: `nr_hugepages` of the 2 MB size.
    pub huge_pages: PathBuf,
    /// The VMM.
    pub vmm: PathBuf,
    /// The host's threads, from its inventory.
    pub threads: Vec<Thread>,
    /// The memory installed, in MiB.
    pub installed_mib: u64,
    /// The host's serial number.
    pub serial: String,
}

fn lock(shared: &Shared) -> MutexGuard<'_, Manager<Machine>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Post through PID 1; a busy moment is waited out.
fn post(control: &Path, code: &str, text: &str) {
    let mut client = supervise::control::Client::new(control);
    for _ in 0..10 {
        if client.post(code.to_string(), text.to_string()).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("PID 1 did not take: {code} {text}");
}

/// The memory the kernel has set aside as huge pages, in MiB.
fn reserved_mib(nr_hugepages: &Path) -> u64 {
    fs::read_to_string(nr_hugepages)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .map_or(0, |pages| pages * HUGE_PAGE_MIB)
}

/// The host as the accounts see it.
fn host(service: &Service, settings: Settings) -> Host {
    let block = settings.memory_block_mib.max(HUGE_PAGE_MIB);
    // What the settings ask for, or less when the kernel gave less.
    let wanted = settings.configurable_mib(service.installed_mib);
    let given = reserved_mib(&service.huge_pages) / block * block;
    Host {
        topology: Topology::of(&service.threads),
        installed_mib: service.installed_mib,
        configurable_mib: wanted.min(given),
        settings,
    }
}

/// The records of the partitions that ran when the last manager ended.
fn records(paths: &Paths) -> Vec<Record> {
    let mut found: Vec<Record> = fs::read_dir(&paths.run)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "record"))
        .filter_map(|entry| {
            let bytes = fs::read(entry.path()).ok()?;
            ipc::decode::<Record>(&bytes).ok()
        })
        .collect();
    found.sort_by_key(|r| r.sequence);
    found
}

/// Take up what runs; what ended while nobody watched is ended now.
fn adopt(shared: &Shared, paths: &Paths, watches: &mpsc::Sender<Watch>) -> usize {
    let mut running = 0;
    for record in records(paths) {
        let group = paths.cgroups.join(resource::cgroup::partition_group(
            record.id,
            &record.current.processors,
        ));
        let report = paths.report(record.id);
        let (lines, read) = watch::lines_of(&report);
        let mut manager = lock(shared);
        manager.adopt(&record, &lines);
        if watch::populated(&group) {
            running += 1;
            let _ = watches.send(Watch {
                id: record.id,
                child: None,
                group,
                report,
                from: read,
            });
        } else {
            let how = watch::end_of(&lines.join("\n"));
            manager.ended(record.id, how);
        }
    }
    running
}

/// One connection: a request and its answer, or a subscription.
fn connection(shared: &Shared, mut stream: UnixStream) -> Result<(), ipc::WireError> {
    stream.set_read_timeout(Some(REQUEST_WAIT))?;
    stream.set_write_timeout(Some(REQUEST_WAIT))?;
    let frame = read_frame_max(&mut stream, MAX_FRAME)?;
    let request: Request = ipc::decode(&frame)?;
    if matches!(request, Request::Subscribe {}) {
        let (events, received) = mpsc::channel();
        lock(shared).subscribe(events);
        write_frame(&mut stream, &ipc::encode(&()))?;
        // Until the subscriber is gone: a write to it then fails.
        for event in received {
            write_frame(&mut stream, &ipc::encode(&event))?;
        }
        return Ok(());
    }
    let answer = proto::dispatch(&mut *lock(shared), request);
    write_frame(&mut stream, &answer)
}

/// Whether PID 1 says the host's base is ready.
fn base_ready(control: &Path) -> bool {
    supervise::control::Client::new(control)
        .status()
        .is_ok_and(|status| status.fact("ready") == Some("yes"))
}

/// Activate the partitions flagged to start with the host, in their
/// order, each after its delay. One that fails does not stop the rest.
fn auto_start(shared: &Shared, control: &Path, marker: &Path) {
    // Once an IPL: the run directory does not outlive one.
    if marker.exists() {
        return;
    }
    let give_up = Instant::now() + BASE_WAIT;
    while !base_ready(control) && Instant::now() < give_up {
        thread::sleep(Duration::from_millis(500));
    }
    let _ = fs::write(marker, "");
    let flagged = lock(shared).auto_start();
    if flagged.is_empty() {
        return;
    }
    let started = &refcode::AUTO_START_STARTED;
    post(
        control,
        started.code,
        &started.text_with(&format!("{} partition(s)", flagged.len())),
    );
    let mut activated = 0;
    for (id, name, delay) in &flagged {
        thread::sleep(Duration::from_secs(u64::from(*delay)));
        let result = lock(shared).activate(*id, String::new(), None, None);
        match result {
            Ok(_) => activated += 1,
            Err(errors) => {
                let failed = &refcode::AUTO_START_FAILED;
                let why = errors.first().map(ToString::to_string).unwrap_or_default();
                post(
                    control,
                    failed.code,
                    &failed.text_with(&format!("{name}, {why}")),
                );
            }
        }
    }
    let done = &refcode::AUTO_START_DONE;
    post(
        control,
        done.code,
        &done.text_with(&format!("{activated} of {} activated", flagged.len())),
    );
}

/// The threads that work beside the socket's: the watchers of the
/// VMMs, the poster, the clock.
fn workers(shared: &Shared, service: &Service, started: Receiver<Watch>) {
    let manager = shared.clone();
    thread::spawn(move || {
        for one in started {
            let shared = manager.clone();
            thread::spawn(move || watch::watch(&shared, one));
        }
    });
    let ticked = shared.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(1));
            lock(&ticked).tick();
        }
    });
    let (manager, control) = (shared.clone(), service.control_socket.clone());
    let marker = service.run_dir.join(RUN).join(AUTO_STARTED);
    thread::spawn(move || auto_start(&manager, &control, &marker));
}

/// Run the service. Returns only when it could not start.
pub fn run(service: &Service) -> io::Error {
    let paths = Paths {
        cgroups: service.cgroups.clone(),
        run: service.run_dir.join(RUN),
        vmm: service.vmm.clone(),
    };
    if let Err(e) = fs::create_dir_all(&paths.run) {
        return e;
    }
    let store = Store::at(&service.state_dir);
    let control = service.control_socket.clone();
    let settings = store.settings().unwrap_or_else(|e| {
        let code = &refcode::PARTITION_STATE_INVALID;
        post(&control, code.code, &code.text_with(&e));
        Settings::default()
    });
    let (watch_to, watches) = mpsc::channel();
    let (post_to, posts) = mpsc::channel::<(String, String)>();
    {
        let control = control.clone();
        thread::spawn(move || {
            for (code, text) in posts {
                post(&control, &code, &text);
            }
        });
    }
    let machine = Machine::new(paths.clone(), watch_to.clone(), post_to);
    let mut manager = Manager::new(machine, store, host(service, settings), &service.serial);
    manager.start();
    let defined = manager.statuses().len();
    let shared: Shared = Arc::new(Mutex::new(manager));
    let running = adopt(&shared, &paths, &watch_to);

    let socket = proto::socket(&service.run_dir.to_string_lossy());
    // One left by the service's last run.
    let _ = fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => return e,
    };
    if let Err(e) = fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)) {
        return e;
    }
    workers(&shared, service, watches);
    if let Err(e) = supervise::ready::signal() {
        return e;
    }
    let started = &refcode::PARTD_STARTED;
    post(
        &control,
        started.code,
        &started.text_with(&format!("{defined} defined, {running} running")),
    );
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let shared = shared.clone();
        thread::spawn(move || {
            // A client that sends no request, or goes, is its own loss.
            let _ = connection(&shared, stream);
        });
    }
    io::Error::other("the socket was closed")
}

/// How a VMM that is no longer there ended, for whoever holds its
/// report's text.
pub fn ended(report: &str) -> Ended {
    watch::end_of(report)
}
