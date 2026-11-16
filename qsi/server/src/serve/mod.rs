// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The server on Linux: what it is given, the listener and each
//! connection's thread, the pumps that feed the event log, and the
//! local socket. The binary reads the identity as root, then drops to
//! the server's own user before [`run`].

pub mod facts;
pub mod listen;
mod local;
pub mod pipes;
pub mod pumps;

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use qsi::{Hello, PeerKind, caps};
use qsisess::Limits;
use rustls::ServerConfig;
use supervise::control::Calls as _;

use crate::events::Store;
use crate::events::hub::{Hub, Shared};
use crate::families::Services;
use crate::trust::Trust;
use facts::{HostFacts, Paths};
use listen::Gate;

/// What the server is given.
#[derive(Clone, Debug)]
pub struct Settings {
    /// What it reads of the host.
    pub paths: Paths,
    /// Its own directory: the consoles and the event log.
    pub own: PathBuf,
    /// Its local socket.
    pub local: PathBuf,
    /// The partition manager's socket.
    pub partd: PathBuf,
    /// The pool service's.
    pub poold: PathBuf,
    /// The switch service's.
    pub vswitchd: PathBuf,
    /// The port.
    pub port: u16,
    /// The release that runs.
    pub release: String,
    /// Its build.
    pub build: String,
    /// The sessions' limits.
    pub limits: Limits,
}

/// The sessions now, for the local socket.
#[derive(Debug, Default)]
pub struct Sessions {
    next: AtomicU64,
    open: Mutex<BTreeMap<u64, (String, String, String)>>,
}

impl Sessions {
    fn open(&self, address: &str, peer: &qsitrust::Peer, trust: &Trust) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let name = if peer.pinned {
            trust
                .consoles()
                .by_print(&peer.fingerprint)
                .map_or_else(String::new, |c| c.name.clone())
        } else {
            "pairing".into()
        };
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, (address.to_string(), name, peer.fingerprint.hex()));
        id
    }

    fn close(&self, id: u64) {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }

    fn connected(&self) -> Vec<String> {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|(_, _, print)| print.clone())
            .collect()
    }

    fn all(&self) -> Vec<(String, String)> {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|(a, n, _)| (a.clone(), n.clone()))
            .collect()
    }
}

/// The server: one for the process, shared by every thread.
pub struct Server {
    settings: Settings,
    identity: qsitrust::Identity,
    tls: Arc<ServerConfig>,
    trust: Trust,
    hub: Shared,
    gate: Mutex<Gate>,
    sessions: Sessions,
    port: u16,
    limits: Limits,
    addresses: Mutex<Vec<Ipv4Addr>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

impl Server {
    /// The server over its identity and settings: the consoles and the
    /// event log taken up from its own directory.
    ///
    /// # Errors
    /// The consoles' file is refused, or TLS refuses the identity.
    pub fn new(settings: Settings, identity: qsitrust::Identity) -> Result<Arc<Self>, String> {
        let now = Arc::new(|| u64::try_from(appliance::time::now_secs()).unwrap_or(0));
        let trust = Trust::open(settings.own.clone(), now)?;
        let tls = qsitrust::tls::server_config(&identity, Arc::new(trust.clone()))
            .map_err(|e| e.to_string())?;
        let (store, restored) = Store::open(&settings.own, &settings.limits);
        let hub = Arc::new(Mutex::new(Hub::kept(store, restored)));
        Ok(Arc::new(Self {
            port: settings.port,
            limits: settings.limits,
            settings,
            identity,
            tls,
            trust,
            hub,
            gate: Mutex::new(Gate::default()),
            sessions: Sessions::default(),
            addresses: Mutex::new(Vec::new()),
        }))
    }

    fn facts(&self) -> HostFacts {
        HostFacts {
            paths: self.settings.paths.clone(),
            release: self.settings.release.clone(),
            build: self.settings.build.clone(),
        }
    }

    /// A line for the system log.
    #[allow(clippy::unused_self)] // every thread says through the server
    fn say(&self, line: &str) {
        say(line);
    }

    /// Post a reference code through PID 1.
    fn post(&self, code: &refcode::RefCode, detail: &str) {
        let text = code.text_with(detail);
        self.say(&format!("{} {text}", code.code));
        let mut c = supervise::control::Client::new(&self.settings.paths.control);
        let _ = c.post(code.code.to_string(), text);
    }

    /// The host's hello.
    fn hello(&self) -> Hello {
        let mut facts = self.facts();
        let me = crate::families::Facts::identity(&mut facts).ok();
        Hello {
            version: qsi::VERSION,
            caps: caps::HOST_V1,
            kind: PeerKind::Host,
            name: me.as_ref().map_or_else(String::new, |m| m.name.clone()),
            release: self.settings.release.clone(),
            serial: me.map_or_else(String::new, |m| m.serial),
        }
    }

    /// A connection's services: each a client of its socket.
    fn services(&self) -> Services {
        Services {
            partd: Box::new(partd::Client::new(&self.settings.partd)),
            poold: Box::new(poold::Client::new(&self.settings.poold)),
            vswitchd: Box::new(vswitchd::Client::new(&self.settings.vswitchd)),
            facts: Box::new(self.facts()),
        }
    }

    fn opener(&self) -> pipes::Sockets {
        pipes::Sockets {
            partd: self.settings.partd.clone(),
            poold: self.settings.poold.clone(),
            hub: self.hub.clone(),
        }
    }

    /// The management addresses now: PID 1's fact.
    fn management(&self) -> Option<Ipv4Addr> {
        let mut c = supervise::control::Client::new(&self.settings.paths.control);
        let status = c.status().ok()?;
        listen::management_address(status.fact(appliance::NOTE_MANAGEMENT)?)
    }

    /// The listener listens on `addresses` now.
    fn listening(&self, addresses: &[Ipv4Addr], first: bool) {
        *self
            .addresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = addresses.to_vec();
        let words: Vec<String> = addresses
            .iter()
            .map(|a| format!("{a}:{}", self.port))
            .collect();
        if addresses.is_empty() {
            self.post(&refcode::QSI_NOT_LISTENING, "no management address");
        } else if first {
            let detail = format!(
                "{}, certificate {}",
                words.join(" "),
                self.identity.fingerprint().hex()
            );
            self.post(&refcode::QSI_STARTED, &detail);
        } else {
            self.post(&refcode::QSI_ADDRESSES, &words.join(" "));
        }
    }

    fn interface(&self) -> qsilocal::Interface {
        let addresses = self
            .addresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        qsilocal::Interface {
            problem: if addresses.is_empty() {
                "There is no management address to listen on.".into()
            } else {
                String::new()
            },
            addresses: addresses
                .iter()
                .map(|a| format!("{a}:{}", self.port))
                .collect(),
            fingerprint: self.identity.fingerprint().hex(),
            sessions: self.sessions.all(),
        }
    }
}

/// A line for the system log: the supervisor takes standard error.
pub fn say(line: &str) {
    eprintln!("{line}");
}

/// Run the server: the pumps, the local socket, the listener. Never
/// returns.
pub fn run(server: &Arc<Server>, local: std::os::unix::net::UnixListener) -> ! {
    pumps::partd(server.settings.partd.clone(), server.hub.clone());
    pumps::vswitchd(server.settings.vswitchd.clone(), server.hub.clone());
    pumps::poold(server.settings.poold.clone(), server.hub.clone());
    pumps::host(server.facts(), server.hub.clone());
    pumps::trust(
        server.trust.clone(),
        server.settings.paths.control.clone(),
        say,
    );
    local::serve(server.clone(), local);
    {
        let hub = server.hub.clone();
        let _ = std::thread::Builder::new()
            .name("sync".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    crate::events::hub::lock(&hub).sync();
                }
            });
    }
    listen::listen(server)
}

#[cfg(test)]
mod tests;
