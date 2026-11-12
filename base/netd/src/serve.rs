// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The network service: the interface machine on its own thread of
//! messages, a thread watching the links, one serving the socket and
//! one keeping time.

use std::convert::Infallible;
use std::io;
use std::os::unix::net::UnixListener;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use appliance::{Appliance, Event, NOTE_MANAGEMENT};
use dhcp::Lease;
use netlink::route::LinkEvents;
use state::{Host, Network, Store};

use crate::control::{Net, NetStatus, Request, TrialStatus, dispatch};
use crate::manage::{Manager, Reporter};
use crate::sys::clock::{SystemClock, write_rtc};
use crate::sys::kernel::Linux;
use crate::time::{Shared, keep};
use crate::trial::{Last, Trial, Verdict};

/// The resolver's file.
const RESOLV: &str = "/etc/resolv.conf";

/// What the machine's thread is told.
pub enum Msg {
    /// The links changed.
    Links,
    /// The DHCP client put this lease on the interface.
    Bound(Lease),
    /// The DHCP client took its lease off.
    Unbound,
    /// A reload was asked for; the answer goes back.
    Reload(Sender<Result<(), String>>),
    /// Status was asked for.
    Status(Sender<NetStatus>),
    /// A change of the network, to be tried.
    Apply(String, Sender<Result<(), String>>),
    /// How the last change stands.
    Trial(Sender<TrialStatus>),
    /// Keep the change on trial.
    Confirm(Sender<Result<(), String>>),
}

/// The state files, read again. A file that is absent is the defaults.
fn load(appliance: &Appliance) -> Result<(Network, Host), String> {
    let store = Store::at(appliance.state_dir());
    let network = store.load::<Network>().map_err(|e| e.to_string())?;
    let host = store.load::<Host>().map_err(|e| e.to_string())?;
    Ok((network.unwrap_or_default(), host.unwrap_or_default()))
}

fn host_name(appliance: &Appliance, host: &Host) -> String {
    let first = appliance::links::ethernet().first().map(|l| l.mac);
    host.name_or_default(appliance.host_prefix, first)
}

/// The socket's side of a request: ask the machine's thread and wait.
struct Asker(Sender<Msg>);

impl Asker {
    fn ask<T>(&self, msg: impl FnOnce(Sender<T>) -> Msg) -> Option<T> {
        let (tx, rx) = mpsc::channel();
        self.0.send(msg(tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }
}

impl Net for Asker {
    fn reload(&mut self) -> Result<(), String> {
        self.ask(Msg::Reload)
            .unwrap_or_else(|| Err("the service did not answer".into()))
    }

    fn status(&mut self) -> NetStatus {
        self.ask(Msg::Status).unwrap_or_default()
    }

    fn apply(&mut self, network: String) -> Result<(), String> {
        self.ask(|reply| Msg::Apply(network, reply))
            .unwrap_or_else(|| Err("the service did not answer".into()))
    }

    fn trial(&mut self) -> TrialStatus {
        self.ask(Msg::Trial).unwrap_or_default()
    }

    fn confirm(&mut self) -> Result<(), String> {
        self.ask(Msg::Confirm)
            .unwrap_or_else(|| Err("the service did not answer".into()))
    }

    fn set_clock(&mut self, secs: i64) -> Result<(), String> {
        // The clock is nobody's state but the kernel's: set here.
        crate::sys::clock::set(secs).map_err(|e| format!("the clock was not set: {e}"))
    }
}

fn serve_socket(listener: &UnixListener, tx: &Sender<Msg>) {
    let mut asker = Asker(tx.clone());
    for stream in listener.incoming().flatten() {
        let _ = supervise::control::answer(stream, |request| {
            Ok(dispatch(&mut asker, ipc::decode::<Request>(request)?))
        });
    }
}

fn watch_links(tx: &Sender<Msg>) {
    loop {
        match LinkEvents::open() {
            Ok(mut events) => {
                while events.wait().is_ok() {
                    if tx.send(Msg::Links).is_err() {
                        return;
                    }
                }
            }
            Err(e) => eprintln!("link events: {e}"),
        }
        // The machine looks every second anyway.
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn manage(
    appliance: &Appliance,
    mut manager: Manager<Linux>,
    rx: &Receiver<Msg>,
    shared: &Mutex<Shared>,
) -> ! {
    // The links are looked at once at the start, then on every event.
    manager.refresh(Instant::now());
    let mut name = host_name(
        appliance,
        &load(appliance).map_or_else(|_| Host::default(), |(_, host)| host),
    );
    let mut trial: Option<Trial> = None;
    let mut last = Last::None;
    loop {
        let msg = rx.recv_timeout(Duration::from_secs(1));
        let now = Instant::now();
        match msg {
            Ok(Msg::Links) | Err(RecvTimeoutError::Timeout) => manager.refresh(now),
            Ok(Msg::Bound(lease)) => manager.bound(&lease),
            Ok(Msg::Unbound) => manager.unbound(now),
            Ok(Msg::Status(reply)) => {
                let _ = reply.send(manager.status());
            }
            Ok(Msg::Apply(text, reply)) => {
                let answer = match (&trial, state::store::parse::<Network>(&text)) {
                    (Some(_), _) => Err("a change is being tried already".to_string()),
                    (None, Err(e)) => Err(e.to_string()),
                    (None, Ok(new)) => {
                        let old = manager.config().clone();
                        manager.reload(new.clone(), name.clone(), now);
                        trial = Some(Trial::begin(old, new, now));
                        last = Last::Trying;
                        Ok(())
                    }
                };
                let _ = reply.send(answer);
            }
            Ok(Msg::Trial(reply)) => {
                let _ = reply.send(TrialStatus {
                    state: last.word().to_string(),
                    seconds_left: trial.as_ref().map_or(0, |t| t.seconds_left(now)),
                    line: manager.line(),
                    reason: match &last {
                        Last::Restored(why) => why.clone(),
                        _ => String::new(),
                    },
                });
            }
            Ok(Msg::Confirm(reply)) => {
                let answer = match &mut trial {
                    Some(t) => {
                        t.confirmed = true;
                        Ok(())
                    }
                    None => Err("no change is being tried".to_string()),
                };
                let _ = reply.send(answer);
            }
            // The files hold the configuration of before the trial.
            Ok(Msg::Reload(reply)) if trial.is_some() => {
                let _ = reply.send(Err("a change is being tried".into()));
            }
            Ok(Msg::Reload(reply)) => {
                let loaded = load(appliance);
                let _ = reply.send(loaded.as_ref().map(|_| ()).map_err(Clone::clone));
                if let Ok((network, host)) = loaded {
                    name = host_name(appliance, &host);
                    shared
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .servers = host.ntp_servers;
                    manager.reload(network, name.clone(), now);
                }
            }
            // Every sender is gone only when the process is ending.
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(Duration::from_secs(1)),
        }
        // A change on trial is kept once it works, and taken back when
        // its time is up.
        if let Some(t) = trial.take() {
            let status = manager.status();
            let working = status.carrier && !status.address.is_empty();
            match t.judge(working, now) {
                Verdict::Wait => trial = Some(t),
                Verdict::Keep => {
                    last = match Store::at(appliance.state_dir()).save(&t.new) {
                        Ok(()) => Last::Kept,
                        Err(e) => {
                            manager.reload(t.old, name.clone(), now);
                            Last::Restored(format!("it could not be written: {e}"))
                        }
                    };
                }
                Verdict::Restore => {
                    let why = status.line;
                    manager.reload(t.old, name.clone(), now);
                    last = Last::Restored(why);
                }
            }
        }
        shared.lock().unwrap_or_else(PoisonError::into_inner).online = manager.online();
    }
}

/// Run the network service of `appliance`. Says it is ready once its
/// socket is bound; an address may come later, or never.
pub fn run(appliance: &'static Appliance) -> io::Result<Infallible> {
    let (network, host) = load(appliance).unwrap_or_else(|e| {
        eprintln!("the configuration is not used, the defaults are: {e}");
        (Network::default(), Host::default())
    });
    let (tx, rx) = mpsc::channel();

    let socket = appliance.net_socket();
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    let for_socket = tx.clone();
    std::thread::spawn(move || serve_socket(&listener, &for_socket));
    let for_links = tx.clone();
    std::thread::spawn(move || watch_links(&for_links));

    let shared = Arc::new(Mutex::new(Shared {
        servers: host.ntp_servers.clone(),
        online: false,
    }));
    let for_time = Arc::clone(&shared);
    std::thread::spawn(move || {
        keep(&for_time, &SystemClock, |server| {
            if let Err(e) = write_rtc() {
                eprintln!("time: the RTC was not written: {e}");
            }
            (appliance.post)(&Event::TimeSynchronized { server });
        })
    });

    let report = Reporter {
        post: Box::new(|event| (appliance.post)(event)),
        note: Box::new(|line| (appliance.note)(NOTE_MANAGEMENT, line)),
    };
    let kernel = Linux::open(tx, RESOLV).map_err(io::Error::other)?;
    let name = host_name(appliance, &host);
    let manager = Manager::new(kernel, report, network, name, Instant::now());
    supervise::ready::signal()?;
    manage(appliance, manager, &rx, &shared)
}
