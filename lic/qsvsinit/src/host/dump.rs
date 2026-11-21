// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's own dump (`docs/DUMPS.md`), asked of PID 1: by a console
//! over QSI, by Service Tools, or by PID 1 itself when a service stops
//! after its retries or a pending slot did not confirm. One at a time;
//! written by a thread of its own, since this loop never waits on
//! `/data`. Its codes are posted through the control socket like any
//! service's.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use hostdump::{Asked, Paths, Store};

use super::Host;

/// The appliance's directories.
fn appliance() -> appliance::Appliance {
    release::appliance(|_| {}, |_, _| {})
}

/// A dump is being taken.
static TAKING: AtomicBool = AtomicBool::new(false);

/// Where the host's dumps are kept.
#[must_use]
pub fn store() -> Store {
    hostdump::store::at(Path::new(release::DATA_DIR).join(hostdump::store::DIR))
}

/// The directory, root's and the QSI server's group's, which lists,
/// reads and deletes them; nobody else's.
fn prepare(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    if rustix::process::geteuid().is_root() {
        std::os::unix::fs::chown(dir, Some(0), Some(release::QSI_USER))?;
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o770))
}

/// A dump written: its group's to read.
fn share(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if rustix::process::geteuid().is_root() {
        let _ = std::os::unix::fs::chown(path, Some(0), Some(release::QSI_USER));
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o640));
}

/// The services' status as a person reads it.
fn status_text(host: &Host) -> String {
    let s = host.status_now();
    let mut out = String::new();
    for (k, v) in &s.facts {
        let _ = writeln!(out, "{k}: {v}");
    }
    out.push('\n');
    for svc in &s.services {
        let _ = writeln!(
            out,
            "{} {} process {} failures {}",
            svc.name, svc.state, svc.pid, svc.failures
        );
    }
    out
}

impl Host {
    /// Begin a dump asked for by `by`: its name, or why not (one is
    /// being taken, or there is no data file system to keep it on).
    pub fn take_dump(&mut self, by: &str) -> Result<String, String> {
        if !self.data_mounted {
            return Err("There is no data file system to keep a dump on.".into());
        }
        if TAKING.swap(true, Ordering::SeqCst) {
            return Err("A host dump is being taken: one at a time.".into());
        }
        let now = appliance::time::now_secs();
        let name = hostdump::store::name_at(now);
        self.post(&refcode::HOST_DUMP_STARTED, &format!("{name}, by {by}"));
        let asked = Asked {
            by: by.to_string(),
            release: format!(
                "{} {} build {}",
                release::NAME,
                release::RELEASE,
                release::BUILD_ID
            ),
            host: format!("{} serial {}", self.facts.host, self.facts.serial),
            status: status_text(self),
        };
        let paths = Paths {
            log: appliance().log_dir(),
            state: appliance().state_dir(),
            problems: Path::new(release::DATA_DIR).join("problems"),
            cgroup: PathBuf::from("/sys/fs/cgroup"),
            proc: PathBuf::from("/proc"),
        };
        let spawned = std::thread::Builder::new()
            .name("host-dump".into())
            .spawn(move || {
                let store = store();
                let mut live = hostdump::live::Live {
                    partd: partd::proto::socket(release::RUN_DIR),
                    poold: poold::proto::socket(release::RUN_DIR),
                    vswitchd: vswitchd::proto::socket(release::RUN_DIR),
                };
                let taken = prepare(store.dir())
                    .map_err(|e| format!("{}: {e}", store.dir().display()))
                    .and_then(|()| hostdump::take(&store, &paths, &mut live, &asked, now));
                match taken {
                    Ok((info, gone)) => {
                        for old in gone {
                            refcode::post::remote::code(
                                &refcode::HOST_DUMP_REMOVED,
                                &format!("{old}, for the limits"),
                            );
                        }
                        if let Some(p) = store.path(&info.name) {
                            share(&p);
                        }
                        refcode::post::remote::code(
                            &refcode::HOST_DUMP_WRITTEN,
                            &format!("{}, {} bytes", info.name, info.bytes),
                        );
                    }
                    Err(e) => refcode::post::remote::code(&refcode::HOST_DUMP_FAILED, &e),
                }
                TAKING.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            TAKING.store(false, Ordering::SeqCst);
            return Err("The dump's thread could not be started.".into());
        }
        Ok(name)
    }

    /// A dump PID 1 asks for itself: said in the log when it cannot be.
    pub fn dump_for(&mut self, why: &str) {
        if let Err(e) = self.take_dump(&format!("PID 1: {why}")) {
            self.line(&format!("no host dump for {why}: {e}"));
        }
    }
}
