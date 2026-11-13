// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The machine behind the manager: the cgroup file system, the VMM's
//! process, PID 1's control socket, the run directory.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use resource::cgroup::Step;
use rustix::process::{Pid, Signal, kill_process};

use super::sys;
use crate::manager::{Record, System};

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
}

/// The machine.
pub struct Machine {
    paths: Paths,
    watches: Sender<Watch>,
    posts: Sender<(String, String)>,
}

impl Machine {
    /// The machine at `paths`. A VMM that is started is sent to
    /// `watches`; a reference code to `posts`.
    pub fn new(paths: Paths, watches: Sender<Watch>, posts: Sender<(String, String)>) -> Self {
        Self {
            paths,
            watches,
            posts,
        }
    }

    fn start(&self, id: u32, group: &str, config: &str) -> io::Result<Child> {
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
        let mut child = sys::spawn_in_group(&mut command, procs)?;
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
        let _ = self.posts.send((code.to_string(), text.to_string()));
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
}
