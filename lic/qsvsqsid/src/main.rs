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
        },
        own,
        local,
        partd: partd::proto::socket(release::RUN_DIR),
        poold: poold::proto::socket(release::RUN_DIR),
        vswitchd: vswitchd::proto::socket(release::RUN_DIR),
        port: release::QSI_PORT,
        release: release::RELEASE.to_string(),
        build: release::BUILD_ID.to_string(),
        limits: qsisess::Limits::default(),
    };
    let server = match qsiserver::serve::Server::new(settings, identity) {
        Ok(s) => s,
        Err(e) => {
            post(&refcode::QSI_NOT_LISTENING, &e);
            std::process::exit(1);
        }
    };
    qsiserver::serve::run(&server, listener)
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsqsid is a service of a QS/VS host and runs on Linux only");
}
