// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's problem service (`docs/PROBLEMS.md`): `svc/problem` run as
//! QS/VS. It reads the reference code history the log service writes,
//! keeps the problem log in `/data/problems`, and shares its socket
//! with the QSI server's group.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::PathBuf;

    let appliance = release::appliance(|_| {}, |_, _| {});
    let host = problem::serve::Host {
        post: refcode::post::remote::code_about,
        run_dir: release::RUN_DIR,
        dir: PathBuf::from(release::DATA_DIR).join("problems"),
        history: appliance.log_dir().join(logd::files::HISTORY),
        data: PathBuf::from(release::DATA_DIR),
        share: release::share_with_server,
    };
    // It returns only when it could not start.
    let e = problem::serve::run(host);
    eprintln!("the problem service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsprobd is a service of a QS/VS host and runs on Linux only");
}
