// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's QSI host server (`docs/QSISERVER.md`): `qsi/server` run as
//! QS/VS. PID 1 starts it last; as root it reads the host's identity,
//! makes its own directory and binds its local socket, then it becomes
//! its own user for good and listens on TCP 2300 of the management
//! addresses.

#[cfg(target_os = "linux")]
mod sys;

#[cfg(target_os = "linux")]
fn main() {
    use std::path::PathBuf;
    use std::time::Duration;

    let appliance = release::appliance(|_| {}, |_, _| {});
    let state = appliance.state_dir();
    let post = refcode::post::remote::code;
    // Ready once it runs: a host with no identity yet waits here, said
    // by its code, rather than failing again and again.
    if let Err(e) = supervise::ready::signal() {
        eprintln!("could not say it is ready: {e}");
    }
    // A processor without what the cryptography needs: the local
    // socket only, and nothing of TLS touched (PID 1 posted A2010010).
    let lacking = qsilocal::processor::lacking();
    if !lacking.is_empty() {
        let problem = qsilocal::processor::problem(&lacking);
        eprintln!("{problem}");
        let own = PathBuf::from(release::DATA_DIR).join("qsi");
        let local = PathBuf::from(release::RUN_DIR).join(qsilocal::SOCKET_FILE);
        let prepared = sys::own_directory(&own, release::QSI_USER)
            .and_then(|()| sys::local_socket(&local))
            .and_then(|listener| sys::drop_privileges(release::QSI_USER).map(|()| listener));
        match prepared {
            Ok(listener) => qsiserver::serve::local::without(&listener, own, problem),
            Err(e) => {
                eprintln!("the QSI server did not start: {e}");
                std::process::exit(1);
            }
        }
    }
    // The identity is made at the first IPL with a data file system: a
    // host without one has nothing to present yet.
    let identity = loop {
        let read = std::fs::read_to_string(state.join(identity::CERT_FILE)).and_then(|cert| {
            let key = std::fs::read_to_string(state.join(identity::KEY_FILE))?;
            Ok((cert, key))
        });
        let made = read.map_err(|e| e.to_string()).and_then(|(cert, key)| {
            qsitrust::Identity::from_pem(&cert, &key).map_err(|e| e.to_string())
        });
        match made {
            Ok(identity) => break identity,
            Err(e) => {
                post(&refcode::QSI_NOT_LISTENING, &format!("no identity: {e}"));
                std::thread::sleep(Duration::from_secs(60));
            }
        }
    };
    let own = PathBuf::from(release::DATA_DIR).join("qsi");
    let local = PathBuf::from(release::RUN_DIR).join(qsilocal::SOCKET_FILE);
    let prepared = sys::own_directory(&own, release::QSI_USER)
        .and_then(|()| sys::local_socket(&local))
        .and_then(|listener| sys::drop_privileges(release::QSI_USER).map(|()| listener));
    let listener = match prepared {
        Ok(l) => l,
        Err(e) => {
            eprintln!("the QSI server did not start: {e}");
            std::process::exit(1);
        }
    };
    let settings = qsiserver::serve::Settings {
        paths: qsiserver::serve::facts::Paths {
            state,
            log: appliance.log_dir(),
            control: appliance.control_socket(),
            dumps: std::path::Path::new(release::DATA_DIR).join(hostdump::store::DIR),
        },
        own,
        local,
        partd: partd::proto::socket(release::RUN_DIR),
        poold: poold::proto::socket(release::RUN_DIR),
        vswitchd: vswitchd::proto::socket(release::RUN_DIR),
        probd: problem::proto::socket(release::ID, release::RUN_DIR),
        updd: update::proto::socket(release::RUN_DIR),
        port: release::QSI_PORT,
        release: release::RELEASE.to_string(),
        build: release::build_shown(),
        limits: qsisess::Limits::default(),
        pairing_secs: pairing_secs(),
    };
    // At the host's shutdown each session says goodbye before the end.
    let term = sys::on_term(|| {
        qsiserver::serve::STOPPING.store(true, std::sync::atomic::Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(500));
        std::process::exit(0);
    });
    if let Err(e) = term {
        eprintln!("SIGTERM is not taken: {e}");
    }
    let server = match qsiserver::serve::Server::new(settings, identity) {
        Ok(s) => s,
        Err(e) => {
            post(&refcode::QSI_NOT_LISTENING, &e);
            std::process::exit(1);
        }
    };
    qsiserver::serve::run(&server, listener)
}

/// How long a pairing password lasts: QSI's 15 minutes; in a test
/// image, what `/run/qsvshook.pairing-secs` says when it is there.
#[cfg(target_os = "linux")]
fn pairing_secs() -> u64 {
    #[cfg(test_hooks)]
    if let Some(secs) = std::fs::read_to_string("/run/qsvshook.pairing-secs")
        .ok()
        .and_then(|t| t.trim().parse().ok())
    {
        return secs;
    }
    qsiserver::trust::LIFETIME_SECS
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsqsid is a service of a QS/VS host and runs on Linux only");
}
