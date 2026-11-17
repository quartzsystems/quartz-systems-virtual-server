// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The machine behind the manager: the cgroup file system, the VMM's
//! process, PID 1's control socket, the run directory.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poold::proto::{Calls as _, Given, Want};
use qvsmsg::Issued;
use resource::cgroup::Step;
use rustix::process::{Pid, Signal, kill_process};
use vswitch::port::PortSpec;
use vswitchd::Calls as _;

use super::channel::Links;
use super::files;
use super::sys;
use crate::manager::{DumpOrder, PhysioFacts, Port, Record, System};
use crate::proto::{DumpInfo, Posted};

/// A VMM to be watched.
pub struct Watch {
    /// The partition.
    pub id: u32,
    /// The process, when this manager started it.
    pub child: Option<Child>,
    /// Its control group, under the cgroup mount.
    pub group: PathBuf,
    /// Its report.
    pub report: PathBuf,
    /// Where in the report to go on reading.
    pub from: u64,
}

/// Where everything is.
#[derive(Clone, Debug)]
pub struct Paths {
    /// The cgroup2 mount.
    pub cgroups: PathBuf,
    /// The partition manager's own directory in the run directory.
    pub run: PathBuf,
    /// The VMM.
    pub vmm: PathBuf,
    /// The pool service's socket.
    pub pools: PathBuf,
    /// The switch service's socket.
    pub switches: PathBuf,
    /// Where each partition's sockets are: `<partitions>/<ID>`.
    pub partitions: PathBuf,
    /// Whether a host path may back a unit: a test image's.
    pub host_paths: bool,
    /// The state directory: each partition's history is in it.
    pub state: PathBuf,
    /// sysfs.
    pub sys: PathBuf,
    /// Where the VFIO container and groups are.
    pub vfio: PathBuf,
    /// A test image's file of addresses whose reset fails.
    pub no_reset: Option<PathBuf>,
}

impl Paths {
    /// The report of the partition `id`'s VMM: its standard error.
    pub fn report(&self, id: u32) -> PathBuf {
        self.run.join(format!("{id}.report"))
    }

    /// What the partition `id` is while it runs.
    pub fn record(&self, id: u32) -> PathBuf {
        self.run.join(format!("{id}.record"))
    }

    /// The listening socket of `id`'s `port`, which its VMM is given.
    pub fn channel(&self, id: u32, port: Port) -> PathBuf {
        let word = match port {
            Port::Service => "service",
            Port::Console => "console",
        };
        self.run.join(format!("{id}.{word}.sock"))
    }

    /// The serial console socket of `id`'s VMM.
    pub fn serial(&self, id: u32) -> PathBuf {
        self.partitions
            .join(id.to_string())
            .join(qsvmm::CONSOLE_SOCKET)
    }
}

/// What a dump's thread tells when it is done.
pub struct Dumped {
    /// The partition.
    pub id: u32,
    /// The dump.
    pub name: String,
    /// Written, or why not.
    pub result: Result<(), String>,
}

/// Where the machine says what happened in the background.
#[derive(Clone)]
pub struct Back {
    /// A VMM whose channel's connections are to be kept.
    pub channels: Sender<u32>,
    /// A dump that is done.
    pub dumped: Sender<Dumped>,
    /// Histories to be written.
    pub history: Sender<(u32, Vec<Posted>)>,
}

/// The machine.
pub struct Machine {
    paths: Paths,
    watches: Sender<Watch>,
    posts: Sender<(String, String, Vec<refcode::about::About>)>,
    back: Back,
    /// The connections of the service channels.
    pub links: Links,
    /// Each partition's channel's listening sockets, not yet handed to
    /// its VMM.
    sockets: BTreeMap<u32, Vec<std::os::unix::net::UnixListener>>,
    /// The files the pool service gave each partition, in order, not
    /// yet handed to its VMM.
    held: BTreeMap<u32, Vec<(Want, File)>>,
    /// The taps the switch service gave each partition, in order: the
    /// VMM's after the pool service's files.
    taps: BTreeMap<u32, Vec<File>>,
    /// The VFIO container and groups opened for each partition: the
    /// VMM's after the channel's sockets.
    vfio: BTreeMap<u32, Vec<File>>,
}

impl Machine {
    /// The machine at `paths`. A VMM that is started is sent to
    /// `watches`; a reference code to `posts`.
    pub fn new(
        paths: Paths,
        watches: Sender<Watch>,
        posts: Sender<(String, String, Vec<refcode::about::About>)>,
        back: Back,
    ) -> Self {
        Self {
            paths,
            watches,
            posts,
            back,
            links: Links::default(),
            sockets: BTreeMap::new(),
            held: BTreeMap::new(),
            taps: BTreeMap::new(),
            vfio: BTreeMap::new(),
        }
    }

    fn pools(&self) -> poold::proto::Client {
        poold::proto::Client::new(&self.paths.pools)
    }

    fn switch_service(&self) -> vswitchd::Client {
        vswitchd::Client::new(&self.paths.switches)
    }

    fn start(&mut self, id: u32, group: &str, config: &str) -> io::Result<Child> {
        fs::create_dir_all(&self.paths.run)?;
        // The report starts anew with each activation. It is appended
        // to, so the VMM cannot write over what it has said.
        let report = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.paths.report(id))?;
        drop(report);
        let report = OpenOptions::new()
            .append(true)
            .open(self.paths.report(id))?;
        let procs = OpenOptions::new()
            .write(true)
            .open(self.paths.cgroups.join(group).join("cgroup.procs"))?;
        let mut command = Command::new(&self.paths.vmm);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::from(report))
            .current_dir("/");
        let files = self
            .held
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .map(|(_, f)| f)
            .chain(self.taps.remove(&id).unwrap_or_default())
            .chain(
                self.sockets
                    .remove(&id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|l| File::from(std::os::fd::OwnedFd::from(l))),
            )
            .chain(self.vfio.remove(&id).unwrap_or_default())
            .collect();
        let mut child = sys::spawn_with(&mut command, procs, files)?;
        // The whole configuration, then the end of its input.
        let written = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no input"))
            .and_then(|mut input| input.write_all(config.as_bytes()));
        if let Err(e) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
        Ok(child)
    }
}

/// One request on the control socket of `id`'s VMM, with `file`; its
/// answer, waited for `wait` at most (for ever with none).
fn exchange(
    paths: &Paths,
    id: u32,
    line: &str,
    file: Option<&File>,
    wait: Option<Duration>,
) -> Result<String, String> {
    let socket = paths
        .partitions
        .join(id.to_string())
        .join(qsvmm::media::CONTROL_SOCKET);
    let talk = || -> io::Result<String> {
        let stream = UnixStream::connect(&socket)?;
        stream.set_read_timeout(wait)?;
        let fds: Vec<_> = file.iter().map(|f| f.as_fd()).collect();
        fdpass::send(&stream, &fds)?;
        (&stream).write_all(format!("{line}\n").as_bytes())?;
        let mut answer = String::new();
        Read::take(&stream, 64 * 1024).read_to_string(&mut answer)?;
        Ok(answer.trim_end().to_string())
    };
    talk().map_err(|e| format!("{}: {e}", socket.display()))
}

/// The dumps directory of `id` in the pool that holds the media
/// repository, by the pool service's word.
fn dump_dir_of(pools: &Path, id: u32) -> Option<PathBuf> {
    let list = poold::proto::Client::new(pools).pools().ok()?;
    let pool = list
        .into_iter()
        .find(|p| p.repository && p.usable() && !p.mount.is_empty())?;
    Some(
        Path::new(&pool.mount)
            .join(files::DUMPS)
            .join(id.to_string()),
    )
}

/// Write a dump: its file made in `dir` as a partial one, handed to the
/// VMM, renamed when whole and removed when not.
fn write_dump(paths: &Paths, id: u32, dir: &Path, order: &DumpOrder) -> Result<(), String> {
    let partial = dir.join(format!("{}{}", order.name, dump::PARTIAL));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| format!("{}: {e}", partial.display()))?;
    let text: String = order
        .text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect();
    let line = format!("dump {} {} {} {text}", order.time, order.code, order.reason);
    let answer = exchange(paths, id, &line, Some(&file), None);
    drop(file);
    match answer {
        Ok(a) if a.starts_with("ok") => fs::rename(&partial, dir.join(&order.name)).map_err(|e| {
            let _ = fs::remove_file(&partial);
            format!("{}: {e}", partial.display())
        }),
        other => {
            let _ = fs::remove_file(&partial);
            Err(other
                .unwrap_or_else(|e| e)
                .trim_start_matches("refused ")
                .to_string())
        }
    }
}

/// Write `value` to a control file.
fn set(path: &Path, value: &str) -> Result<(), String> {
    let mut file: File = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(value.as_bytes())
        .map_err(|e| format!("{} = {value}: {e}", path.display()))
}

impl System for Machine {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn apply(&mut self, plan: &[Step]) -> Result<(), String> {
        for step in plan {
            match step {
                Step::Dir(dir) => {
                    let path = self.paths.cgroups.join(dir);
                    match fs::create_dir(&path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                        Err(e) => return Err(format!("{}: {e}", path.display())),
                    }
                }
                Step::Write(file, value) => set(&self.paths.cgroups.join(file), value)?,
            }
        }
        Ok(())
    }

    fn remove_group(&mut self, group: &str) {
        let path = self.paths.cgroups.join(group);
        // A process that has just ended may be a moment in leaving.
        for _ in 0..50 {
            match fs::remove_dir(&path) {
                Ok(()) => return,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return,
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
        println!("the control group {group} was not removed");
    }

    fn launch(&mut self, id: u32, group: &str, config: &str) -> Result<u32, String> {
        let child = self
            .start(id, group, config)
            .map_err(|e| format!("{}: {e}", self.paths.vmm.display()))?;
        let pid = child.id();
        let watch = Watch {
            id,
            child: Some(child),
            group: self.paths.cgroups.join(group),
            report: self.paths.report(id),
            from: 0,
        };
        self.watches
            .send(watch)
            .map_err(|_| "nothing watches the VMMs".to_string())?;
        let _ = self.back.channels.send(id);
        Ok(pid)
    }

    fn end(&mut self, pid: u32) {
        if let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) {
            let _ = kill_process(pid, Signal::TERM);
        }
    }

    fn kill(&mut self, group: &str) {
        let path = self.paths.cgroups.join(group).join("cgroup.kill");
        if let Err(e) = set(&path, "1") {
            println!("{e}");
        }
    }

    fn post(&mut self, code: &str, text: &str) {
        // Sent by a thread of its own: PID 1 may be a moment in
        // answering, and the manager is not held for it.
        let _ = self
            .posts
            .send((code.to_string(), text.to_string(), Vec::new()));
    }

    fn post_about(&mut self, code: &str, text: &str, about: &[refcode::about::About]) {
        let _ = self
            .posts
            .send((code.to_string(), text.to_string(), about.to_vec()));
    }

    fn record(&mut self, id: u32, record: Option<&Record>) {
        let path = self.paths.record(id);
        let done = match record {
            Some(record) => fs::create_dir_all(&self.paths.run)
                .and_then(|()| state::store::atomic_write(&path, &ipc::encode(record))),
            None => match fs::remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        };
        if let Err(e) = done {
            println!("{}: {e}", path.display());
        }
    }

    fn log(&mut self, line: &str) {
        // Standard output reaches the host's log through PID 1.
        println!("{line}");
    }

    fn exists(&self, backing: &str) -> bool {
        Path::new(backing).exists()
    }

    fn hold(&mut self, id: u32, wants: &[Want]) -> Result<Vec<Given>, Vec<Issued>> {
        let unreachable = |e: &dyn std::fmt::Display| {
            vec![qvsmsg::POOLS_UNREACHABLE.with(&[&id.to_string(), &e.to_string()])]
        };
        let given = self
            .pools()
            .hold_files(id, wants.to_vec())
            .map_err(|e| unreachable(&e))??;
        let held = self.held.entry(id).or_default();
        let mut out = Vec::with_capacity(given.len());
        for (g, file) in given {
            held.retain(|(w, _)| !(w.kind == g.want.kind && w.slot == g.want.slot));
            held.push((g.want.clone(), file));
            out.push(g);
        }
        Ok(out)
    }

    fn give_back(&mut self, id: u32, wants: &[Want]) {
        if wants.is_empty() {
            self.held.remove(&id);
        } else if let Some(held) = self.held.get_mut(&id) {
            held.retain(|(w, _)| !wants.iter().any(|x| x.kind == w.kind && x.slot == w.slot));
        }
        if let Err(e) = self.pools().release(id, wants.to_vec()) {
            println!("the pool service did not take back partition {id}'s: {e}");
        }
    }

    fn control(&mut self, id: u32, line: &str, with: Option<&Want>) -> Result<String, String> {
        let file = with.and_then(|w| {
            let held = self.held.get_mut(&id)?;
            let at = held
                .iter()
                .position(|(x, _)| x.kind == w.kind && x.slot == w.slot)?;
            Some(held.remove(at).1)
        });
        exchange(
            &self.paths,
            id,
            line,
            file.as_ref(),
            Some(Duration::from_secs(10)),
        )
    }

    fn host_paths(&self) -> bool {
        self.paths.host_paths
    }

    fn delete_vdisks(&mut self, disks: &[(String, String)]) -> Vec<Issued> {
        let mut client = self.pools();
        let mut refused = Vec::new();
        for (pool, name) in disks {
            match client.delete_vdisk(pool.clone(), name.clone(), true) {
                Ok(Ok(())) => {}
                Ok(Err(mut r)) => refused.append(&mut r),
                Err(e) => refused.push(
                    qvsmsg::STORAGE_FAILED.with(&["delete the virtual disk", &e.to_string()]),
                ),
            }
        }
        refused
    }

    fn switches(&mut self) -> Option<Vec<vswitch::Switch>> {
        self.switch_service()
            .switches()
            .ok()
            .map(|all| all.into_iter().filter_map(|s| s.switch).collect())
    }

    fn open_port(&mut self, id: u32, spec: &PortSpec) -> Result<u32, Vec<Issued>> {
        match self.switch_service().open_port_file(spec.clone()) {
            Ok(Ok((given, file))) => {
                self.taps.entry(id).or_default().push(file);
                Ok(given.mtu)
            }
            Ok(Err(refused)) => Err(refused),
            Err(e) => Err(vec![
                qvsmsg::SWITCHES_UNREACHABLE.with(&[&id.to_string(), &e.to_string()]),
            ]),
        }
    }

    fn close_ports(&mut self, id: u32) {
        self.taps.remove(&id);
        if let Err(e) = self.switch_service().close_port(id, 0) {
            println!("the switch service did not close partition {id}'s ports: {e}");
        }
    }

    fn reconcile_ports(&mut self, running: &[u32]) {
        match self.switch_service().reconcile(running.to_vec()) {
            Ok(0) => {}
            Ok(n) => {
                println!("the switch service closed {n} port(s) of partitions that do not run");
            }
            Err(e) => println!("the switch service did not reconcile its ports: {e}"),
        }
    }

    fn channel_sockets(
        &mut self,
        id: u32,
        first: usize,
    ) -> Result<Vec<qsvmm::config::ConsolePort>, String> {
        let mut listeners = Vec::new();
        let mut ports = Vec::new();
        for (i, (port, name)) in [
            (Port::Service, qsipart::SERVICE_PORT),
            (Port::Console, qsipart::CONSOLE_PORT),
        ]
        .into_iter()
        .enumerate()
        {
            let path = self.paths.channel(id, port);
            let _ = fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .map_err(|e| format!("{}: {e}", path.display()))?;
            listeners.push(listener);
            ports.push(qsvmm::config::ConsolePort {
                name: name.to_string(),
                socket: format!("/proc/self/fd/{}", first + i).into(),
            });
        }
        self.sockets.insert(id, listeners);
        Ok(ports)
    }

    fn send(&mut self, id: u32, port: Port, frame: Vec<u8>) {
        self.links.send(id, port, frame);
    }

    fn close_channel(&mut self, id: u32, port: Port) {
        self.links.close(id, port);
    }

    fn keep(&mut self, id: u32) -> Result<(), String> {
        match self.control(id, "hold", None)? {
            a if a.starts_with("ok") => Ok(()),
            a => Err(a.trim_start_matches("refused ").to_string()),
        }
    }

    fn dump_dir(&mut self, id: u32) -> Option<PathBuf> {
        dump_dir_of(&self.paths.pools, id)
    }

    fn begin_dump(&mut self, id: u32, order: &DumpOrder) -> Result<(), String> {
        let dir = dump_dir_of(&self.paths.pools, id)
            .ok_or_else(|| "no pool holds the media repository".to_string())?;
        fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        files::clear_partial(&dir);
        let (paths, order, back) = (self.paths.clone(), order.clone(), self.back.dumped.clone());
        thread::Builder::new()
            .name("dump".into())
            .spawn(move || {
                let result = write_dump(&paths, id, &dir, &order);
                let _ = back.send(Dumped {
                    id,
                    name: order.name.clone(),
                    result,
                });
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn dumps(&mut self, id: u32) -> Vec<DumpInfo> {
        dump_dir_of(&self.paths.pools, id)
            .map(|dir| files::dumps_in(&dir))
            .unwrap_or_default()
    }

    fn remove_dump(&mut self, id: u32, name: &str) -> bool {
        if !crate::manager::valid_dump_name(name) {
            return false;
        }
        dump_dir_of(&self.paths.pools, id)
            .is_some_and(|dir| fs::remove_file(dir.join(name)).is_ok())
    }

    fn keep_history(&mut self, id: u32, history: &[Posted]) {
        let _ = self.back.history.send((id, history.to_vec()));
    }

    fn kept_history(&mut self, id: u32) -> Vec<Posted> {
        files::read_history(&files::history_path(&self.paths.state, id))
    }

    fn physio_facts(&mut self) -> PhysioFacts {
        let roots = hostinv::Roots {
            sys: self.paths.sys.clone(),
            proc: PathBuf::from("/proc"),
        };
        let mut uses = pio::HostUses::default();
        if let Ok(disks) = self.pools().disks() {
            for d in disks {
                if d.what.starts_with("Load source") {
                    uses.load_source.push(d.location.clone());
                }
                if !d.pool.is_empty() {
                    uses.pool_members.push((d.pool.clone(), d.location.clone()));
                }
            }
        }
        if let Ok(interfaces) = self.switch_service().interfaces() {
            for i in interfaces {
                if i.management {
                    uses.management.push(i.location.clone());
                }
                if !i.switch.is_empty() {
                    uses.uplinks.push((i.switch.clone(), i.location.clone()));
                }
            }
        }
        PhysioFacts {
            pci: hostinv::pci(&roots),
            iommu: hostinv::iommu(&roots),
            uses,
        }
    }

    fn bind(&mut self, address: &str, passthrough: bool) -> Result<(), String> {
        if passthrough {
            pio::bind::to_passthrough(&self.paths.sys, address)
        } else {
            pio::bind::to_host(&self.paths.sys, address)
        }
    }

    fn reset(&mut self, address: &str) -> Result<(), String> {
        if let Some(file) = &self.paths.no_reset
            && fs::read_to_string(file).is_ok_and(|t| t.lines().any(|l| l.trim() == address))
        {
            return Err("the test hooks made its reset fail".into());
        }
        pio::bind::reset(&self.paths.sys, address)
    }

    fn open_vfio(
        &mut self,
        id: u32,
        groups: &[u32],
        first: usize,
    ) -> Result<(String, Vec<(u32, String)>), String> {
        let open = |path: PathBuf| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))
        };
        let mut files = vec![open(self.paths.vfio.join("vfio"))?];
        let mut paths = Vec::new();
        for (i, g) in groups.iter().enumerate() {
            files.push(open(self.paths.vfio.join(g.to_string()))?);
            paths.push((*g, format!("/proc/self/fd/{}", first + 1 + i)));
        }
        self.vfio.insert(id, files);
        Ok((format!("/proc/self/fd/{first}"), paths))
    }

    fn keep_fenced(&mut self, fenced: &[String]) {
        let mut text = fenced.join("\n");
        text.push('\n');
        if let Err(e) = state::atomic_write(&self.paths.run.join(FENCED), text.as_bytes()) {
            self.log(&format!("the fenced devices were not kept: {e}"));
        }
    }

    fn fenced(&mut self) -> Vec<String> {
        fs::read_to_string(self.paths.run.join(FENCED))
            .map(|t| {
                t.lines()
                    .map(str::to_string)
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The devices fenced since the IPL, in the run directory: a manager
/// started again keeps them, a host restarted forgets them.
const FENCED: &str = "fenced";
