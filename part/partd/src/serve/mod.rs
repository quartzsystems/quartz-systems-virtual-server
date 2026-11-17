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
//! 4. The socket is bound, `C2006000` posted, and the service says it
//!    is ready: at the host's IPL the code so comes before the base is
//!    ready.
//! 5. When the host's base is ready, the partitions flagged to start
//!    with it are activated, once an IPL.
//!
//! - [`machine`]: what the manager asks of the host.
//! - [`watch`]: a VMM's report and its end.
//! - [`channel`]: the connections of each service channel.
//! - [`console`]: a console's viewer on the socket.
//! - [`files`]: histories and dumps on disk.
//! - [`sys`]: starting a process in its control group.

pub mod channel;
mod console;
pub mod files;
pub mod machine;
mod sg;
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

use crate::manager::{Answer, Ended, Manager, Port, Record, System as _};
use crate::proto::{self, MAX_FRAME, Partitions as _, RUN, Refused, Request};
use crate::store::Store;
use machine::{Back, Dumped, Machine, Paths, Watch};
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
    /// The pool service's socket.
    pub pools: PathBuf,
    /// The switch service's socket.
    pub switches: PathBuf,
    /// Whether a host path may back a unit: a test image's.
    pub host_paths: bool,
    /// A file of PCI addresses whose reset is made to fail: a test
    /// image's alone.
    pub no_reset: Option<PathBuf>,
}

/// How often the pools are asked whether one failed.
const POOL_LOOK: Duration = Duration::from_secs(2);

fn lock(shared: &Shared) -> MutexGuard<'_, Manager<Machine>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Post through PID 1; a busy moment is waited out.
fn post(control: &Path, code: &str, text: &str) {
    post_about(control, code, text, &[]);
}

/// Post through PID 1 with what the code is about.
fn post_about(control: &Path, code: &str, text: &str, about: &[refcode::about::About]) {
    let mut client = supervise::control::Client::new(control);
    for _ in 0..10 {
        if client
            .post_about(code.to_string(), text.to_string(), about.to_vec())
            .is_ok()
        {
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
fn adopt(
    shared: &Shared,
    paths: &Paths,
    watches: &mpsc::Sender<Watch>,
    channels: &mpsc::Sender<u32>,
) -> usize {
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
            let _ = channels.send(record.id);
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

/// What a request that waits for the system answers: its result, or
/// that the system did not answer in time.
fn waited(answer: Answer, name: &str) -> Result<String, Refused> {
    match answer {
        Answer::Now(result) => result,
        Answer::Later(rx) => rx
            .recv_timeout(Duration::from_secs(svcchan::ANSWER_SECS + 5))
            .unwrap_or_else(|_| {
                Err(vec![
                    qvsmsg::SYSTEM_SILENT.with(&[name, &svcchan::ANSWER_SECS.to_string()]),
                ])
            }),
    }
}

/// A dump's bytes on the connection, after its size.
fn read_dump(
    shared: &Shared,
    mut stream: UnixStream,
    id: u32,
    name: &str,
) -> Result<(), ipc::WireError> {
    let (size, dir) = {
        let mut manager = lock(shared);
        (
            manager.read_dump(id, name.to_string()),
            manager.system().dump_dir(id),
        )
    };
    write_frame(&mut stream, &ipc::encode(&size))?;
    let (Ok(_), Some(dir)) = (size, dir) else {
        return Ok(());
    };
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut file = fs::File::open(dir.join(name))?;
    console::send_file(&mut stream, &mut file)?;
    Ok(())
}

/// One connection: a request and its answer, or a subscription, a
/// console's viewer, a dump's bytes.
fn connection(
    shared: &Shared,
    paths: &Paths,
    mut stream: UnixStream,
) -> Result<(), ipc::WireError> {
    stream.set_read_timeout(Some(REQUEST_WAIT))?;
    stream.set_write_timeout(Some(REQUEST_WAIT))?;
    let frame = read_frame_max(&mut stream, MAX_FRAME)?;
    let request: Request = ipc::decode(&frame)?;
    match request {
        Request::ShutDownAs {
            id,
            kind,
            delay,
            restart,
        } => {
            let (answer, name) = {
                let mut manager = lock(shared);
                let name = manager.name_of(id);
                (manager.begin_shut_down(id, kind, delay, restart), name)
            };
            let result = waited(answer, &name).map(|_| ());
            return write_frame(&mut stream, &ipc::encode(&result));
        }
        Request::Panel { id, function, args } => {
            let (answer, name) = {
                let mut manager = lock(shared);
                let name = manager.name_of(id);
                (manager.begin_panel(id, function, args), name)
            };
            return write_frame(&mut stream, &ipc::encode(&waited(answer, &name)));
        }
        Request::Console { id, take_over } => {
            return console::serve(shared, &paths.serial(id), stream, id, take_over);
        }
        Request::ReadDump { id, name } => return read_dump(shared, stream, id, &name),
        _ => {}
    }
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
            Ok(_) => {
                activated += 1;
                // The next waits for this one's IPL to complete, or the
                // time its definition gives.
                let wait = lock(shared).auto_wait(*id);
                if let Some(secs) = wait {
                    let give_up = Instant::now() + Duration::from_secs(u64::from(secs));
                    while !lock(shared).ipl_complete(*id) && Instant::now() < give_up {
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
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

/// What the machine says in the background, taken up: a VMM's channel
/// to be kept, a dump that is done.
fn background(shared: &Shared, paths: &Paths, channels: Receiver<u32>, dumped: Receiver<Dumped>) {
    let (manager, paths) = (shared.clone(), paths.clone());
    thread::spawn(move || {
        for id in channels {
            let links = lock(&manager).system().links.clone();
            let sockets = [
                (Port::Service, paths.channel(id, Port::Service)),
                (Port::Console, paths.channel(id, Port::Console)),
            ];
            channel::watch(&manager, &links, id, sockets);
        }
    });
    let manager = shared.clone();
    thread::spawn(move || {
        for d in dumped {
            lock(&manager).dumped(d.id, &d.name, d.result);
        }
    });
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
    let (watched, pools, switches) = (
        shared.clone(),
        service.pools.clone(),
        service.switches.clone(),
    );
    thread::spawn(move || {
        let mut client = poold::proto::Client::new(&pools);
        let switch_service = vswitchd::Client::new(&switches);
        loop {
            thread::sleep(POOL_LOOK);
            if let Ok(all) = vswitchd::Calls::switches(&mut switch_service.clone()) {
                let down: Vec<String> = all
                    .into_iter()
                    .filter(|s| s.uplink_state == vswitchd::uplink_state::DOWN)
                    .filter_map(|s| s.switch.map(|s| s.name))
                    .collect();
                lock(&watched).uplinks(&down);
            }
            let Ok(list) = poold::proto::Calls::pools(&mut client) else {
                continue;
            };
            for pool in list
                .iter()
                .filter(|p| p.state == poold::proto::state::FAILED)
            {
                lock(&watched).pool_failed(&pool.name);
            }
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
        pools: service.pools.clone(),
        switches: service.switches.clone(),
        partitions: service.run_dir.join("partitions"),
        host_paths: service.host_paths,
        state: service.state_dir.clone(),
        sys: PathBuf::from("/sys"),
        vfio: PathBuf::from("/dev/vfio"),
        no_reset: service.no_reset.clone(),
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
    let (post_to, posts) = mpsc::channel::<(String, String, Vec<refcode::about::About>)>();
    {
        let control = control.clone();
        thread::spawn(move || {
            for (code, text, about) in posts {
                post_about(&control, &code, &text, &about);
            }
        });
    }
    let (channels_to, channels) = mpsc::channel();
    let (dumped_to, dumped) = mpsc::channel();
    let back = Back {
        channels: channels_to.clone(),
        dumped: dumped_to,
        history: files::history_writer(service.state_dir.clone()),
    };
    let machine = Machine::new(paths.clone(), watch_to.clone(), post_to, back);
    let mut manager = Manager::new(machine, store, host(service, settings), &service.serial);
    manager.start();
    let defined = manager.statuses().len();
    let shared: Shared = Arc::new(Mutex::new(manager));
    background(&shared, &paths, channels, dumped);
    let running = adopt(&shared, &paths, &watch_to, &channels_to);
    // The switch service closes the ports of partitions that ended while
    // no partition manager watched.
    {
        let mut manager = lock(&shared);
        let ids: Vec<u32> = manager
            .statuses()
            .into_iter()
            .filter(|s| s.state.active())
            .map(|s| s.id)
            .collect();
        manager.system().reconcile_ports(&ids);
    }

    let socket = proto::socket(&service.run_dir.to_string_lossy());
    // One left by the service's last run.
    let _ = fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => return e,
    };
    if let Err(e) = fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .and_then(|()| release::share_with_server(&socket))
    {
        return e;
    }
    workers(&shared, service, watches);
    // Posted before the service says it is ready, so at the host's IPL
    // it comes before the base is ready and never after it.
    let started = &refcode::PARTD_STARTED;
    post(
        &control,
        started.code,
        &started.text_with(&format!("{defined} defined, {running} running")),
    );
    if let Err(e) = supervise::ready::signal() {
        return e;
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let (shared, paths) = (shared.clone(), paths.clone());
        thread::spawn(move || {
            // A client that sends no request, or goes, is its own loss.
            let _ = connection(&shared, &paths, stream);
        });
    }
    io::Error::other("the socket was closed")
}

/// How a VMM that is no longer there ended, for whoever holds its
/// report's text.
pub fn ended(report: &str) -> Ended {
    watch::end_of(report)
}
