// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The switch service on Linux. In order:
//!
//! 1. The host forwards nothing: IPv4 and IPv6 forwarding are off.
//! 2. Every switch of `vswitch.toml` is built, what the kernel has
//!    already kept; the ports whose taps are still there are taken up.
//! 3. When the partition manager answers, the ports of partitions that
//!    do not run are closed: the service was started again.
//! 4. The socket is bound and the service says it is ready.
//! 5. Every second, the ports whose VMMs ended are forgotten and the
//!    uplinks' carrier looked at.
//!
//! At `SIGTERM` it ends and leaves the kernel's objects as they are:
//! PID 1 has ended every partition before, and a service started again
//! finds them.

use std::fs;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use refcode::RefCode;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use supervise::signal::{Signals, TERM};
use vswitch::port::PortSpec;
use vswitch::{Limits, Switches, Uplink};

use crate::kernel::linux::Linux;
use crate::manager::{Core, Env};
use crate::proto::{
    self, Adapter, Event, FdbRow, Given, Interface, Nic, PortStatus, Refused, Request,
    SwitchStatus, VlanUse,
};

/// What the service is given by the product it runs as.
#[derive(Clone, Copy)]
pub struct Host {
    /// Post a reference code.
    pub post: fn(&'static RefCode, &str),
    /// The run directory: the socket and the ports' record.
    pub run_dir: &'static str,
    /// The state directory: `vswitch.toml`.
    pub state_dir: &'static str,
    /// The host's physical interfaces, from its inventory.
    pub nics: fn() -> Vec<Nic>,
    /// The management network's device, from the network service.
    pub management: fn() -> Option<String>,
    /// Every profile's Ethernet adapter, from the partition manager.
    pub adapters: fn() -> Result<Vec<Adapter>, String>,
    /// The partitions activated now, from the partition manager; none
    /// when it does not answer.
    pub running: fn() -> Option<Vec<u32>>,
}

/// How often the loop looks.
const TURN: Duration = Duration::from_secs(1);

/// The host behind the core.
pub struct Machine {
    host: Host,
}

impl Machine {
    fn ports_file(&self) -> PathBuf {
        Path::new(self.host.run_dir).join("vswitchd").join("ports")
    }

    /// What the run directory says of the ports.
    fn kept_ports(&self) -> Vec<PortSpec> {
        fs::read(self.ports_file())
            .ok()
            .and_then(|b| ipc::decode(&b).ok())
            .unwrap_or_default()
    }
}

impl Env for Machine {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn post(&mut self, code: &'static RefCode, detail: &str) {
        (self.host.post)(code, detail);
    }

    fn log(&mut self, line: &str) {
        // Standard output reaches the host's log through PID 1.
        println!("{line}");
    }

    fn nics(&mut self) -> Vec<Nic> {
        (self.host.nics)()
    }

    fn management(&mut self) -> Option<String> {
        (self.host.management)()
    }

    fn adapters(&mut self) -> Result<Vec<Adapter>, String> {
        (self.host.adapters)()
    }

    fn save(&mut self, switches: &Switches) -> Result<(), String> {
        state::Store::at(self.host.state_dir)
            .save(switches)
            .map_err(|e| e.to_string())
    }

    fn save_ports(&mut self, ports: &[PortSpec]) {
        let path = self.ports_file();
        let done = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| state::store::atomic_write(&path, &ipc::encode(&ports.to_vec())));
        if let Err(e) = done {
            println!("{}: {e}", path.display());
        }
    }
}

/// The service, shared by the loop and the connections.
type Shared = Arc<Mutex<Core<Linux, Machine>>>;

fn lock(core: &Shared) -> MutexGuard<'_, Core<Linux, Machine>> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Session<'a> {
    core: &'a Shared,
}

impl proto::Switches for Session<'_> {
    fn switches(&mut self) -> Vec<SwitchStatus> {
        lock(self.core).statuses()
    }
    fn create(&mut self, name: String, vlans: Vec<u16>, mtu: u32) -> Result<SwitchStatus, Refused> {
        lock(self.core).create(&name, &vlans, mtu)
    }
    fn change(&mut self, name: String, vlans: Vec<u16>, mtu: u32) -> Result<(), Refused> {
        lock(self.core).change(&name, &vlans, mtu)
    }
    fn delete(&mut self, name: String, force: bool) -> Result<(), Refused> {
        lock(self.core).delete(&name, force)
    }
    fn rename(&mut self, name: String, to: String) -> Result<(), Refused> {
        lock(self.core).rename(&name, &to)
    }
    fn set_uplink(&mut self, name: String, uplink: Uplink) -> Result<(), Refused> {
        lock(self.core).set_uplink(&name, &uplink)
    }
    fn clear_uplink(&mut self, name: String) -> Result<(), Refused> {
        lock(self.core).clear_uplink(&name)
    }
    fn interfaces(&mut self) -> Vec<Interface> {
        lock(self.core).interfaces()
    }
    fn ports(&mut self, switch: String) -> Vec<PortStatus> {
        lock(self.core).port_statuses(&switch)
    }
    fn fdb(&mut self, switch: String) -> Result<Vec<FdbRow>, Refused> {
        lock(self.core).fdb(&switch)
    }
    fn vlans(&mut self) -> Vec<VlanUse> {
        lock(self.core).vlan_use()
    }
    fn open_port(&mut self, spec: PortSpec) -> Result<Given, Refused> {
        // Answered with its file by the connection itself.
        let _ = spec;
        Err(Vec::new())
    }
    fn close_port(&mut self, partition: u32, slot: u32) -> u32 {
        lock(self.core).close_port(partition, slot)
    }
    fn limits(&mut self) -> Limits {
        lock(self.core).switches().limits
    }
    fn change_limits(&mut self, limits: Limits) -> Result<(), Refused> {
        lock(self.core).change_limits(limits)
    }
    fn subscribe(&mut self) {}
    fn reconcile(&mut self, running: Vec<u32>) -> u32 {
        lock(self.core).reconcile(&running)
    }
}

fn answer(stream: &UnixStream, bytes: &[u8]) -> io::Result<()> {
    let mut s = stream;
    ipc::frame::write_frame(&mut s, bytes).map_err(io::Error::other)
}

/// One connection: a request and its answer, or a subscription.
fn connection(core: &Shared, stream: &UnixStream) {
    let mut s = stream;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Ok(request) = ipc::frame::read_frame_max(&mut s, proto::MAX_FRAME) else {
        return;
    };
    let Ok(request) = ipc::decode::<Request>(&request) else {
        return;
    };
    let done = match request {
        Request::OpenPort { spec } => {
            let opened = lock(core).open_port(&spec);
            match opened {
                Ok((given, file)) => answer(stream, &ipc::encode(&Ok::<_, Refused>(given)))
                    .and_then(|()| fdpass::send(stream, &[file.as_fd()])),
                Err(r) => answer(stream, &ipc::encode(&Err::<Given, Refused>(r))),
            }
        }
        Request::Subscribe {} => {
            let (tx, rx) = mpsc::channel::<Event>();
            lock(core).subscribe(tx);
            let mut ok = answer(stream, &ipc::encode(&()));
            while ok.is_ok() {
                match rx.recv() {
                    Ok(event) => ok = answer(stream, &ipc::encode(&event)),
                    Err(_) => break,
                }
            }
            Ok(())
        }
        other => answer(stream, &proto::dispatch(&mut Session { core }, other)),
    };
    if let Err(e) = done {
        eprintln!("a connection: {e}");
    }
}

/// Whether `SIGTERM` came within `wait`.
fn term(signals: &mut Signals, wait: Duration) -> bool {
    let timeout = Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap_or(1),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    let mut fds = [PollFd::from_borrowed_fd(signals.fd(), PollFlags::IN)];
    let readable = poll(&mut fds, Some(&timeout)).is_ok() && !fds[0].revents().is_empty();
    readable && signals.read().contains(&TERM)
}

/// Run the switch service for `host`. It returns only when it could
/// not start.
pub fn run(host: Host) -> String {
    let mut signals = match Signals::take(&[TERM]) {
        Ok(signals) => signals,
        Err(e) => return format!("the signal descriptor: {e}"),
    };
    // Nothing is forwarded between switches or to the management
    // network: the host routes nothing.
    let sysctl = netlink::sysctl::Sysctl::default();
    for key in ["ipv4/ip_forward", "ipv6/conf/all/forwarding"] {
        if let Err(e) = sysctl.write(key, "0") {
            println!("{key}: {e}");
        }
    }
    let kernel = match Linux::open() {
        Ok(kernel) => kernel,
        Err(e) => return format!("netlink: {e}"),
    };
    let (state, sweep) = match state::Store::at(host.state_dir).load::<Switches>() {
        Ok(state) => (state.unwrap_or_default(), true),
        Err(e) => {
            // What the kernel has is left alone: a file that is wrong
            // takes nothing away.
            (host.post)(
                &refcode::SWITCH_NOT_BUILT,
                &format!("{}: {e}", <Switches as state::store::Document>::FILE),
            );
            (Switches::default(), false)
        }
    };
    let machine = Machine { host };
    let kept = machine.kept_ports();
    let mut core = Core::new(kernel, machine, state);
    core.start(&kept, sweep);
    if let Some(running) = (host.running)() {
        core.reconcile(&running);
    }
    let socket = proto::socket(host.run_dir);
    let _ = fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => return format!("{}: {e}", socket.display()),
    };
    // The QSI host server asks too.
    if let Err(e) = release::share_with_server(&socket) {
        return format!("{}: {e}", socket.display());
    }
    let core: Shared = Arc::new(Mutex::new(core));
    let for_socket = core.clone();
    let _ = std::thread::Builder::new()
        .name("socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let core = for_socket.clone();
                let _ = std::thread::Builder::new()
                    .name("connection".into())
                    .spawn(move || connection(&core, &stream));
            }
        });
    if let Err(e) = supervise::ready::signal() {
        return format!("the readiness pipe: {e}");
    }
    let started = Instant::now();
    loop {
        if term(&mut signals, TURN) {
            std::process::exit(0);
        }
        lock(&core).turn(started.elapsed().as_secs());
    }
}
