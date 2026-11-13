// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvmm --profile <file> [--data <dir>] [--run <dir>]`: run the
//! partition of the profile until it ends, and exit with the code
//! that says why (`qsvmm::end`).

use std::path::PathBuf;
use std::process::ExitCode;

/// The exit code of a command line that is not one.
const USAGE: u8 = 2;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    use qsvmm::run::{Options, run};

    let mut profile = None;
    let mut data_dir = PathBuf::from(release::DATA_DIR);
    let mut run_dir = PathBuf::from(release::RUN_DIR);
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().map(PathBuf::from);
        match (arg.to_str(), value) {
            (Some("--profile"), Some(value)) => profile = Some(value),
            (Some("--data"), Some(value)) => data_dir = value,
            (Some("--run"), Some(value)) => run_dir = value,
            _ => profile = None,
        }
    }
    let Some(profile) = profile else {
        eprintln!("usage: qsvmm --profile <file> [--data <dir>] [--run <dir>]");
        return ExitCode::from(USAGE);
    };
    let end = run(&Options {
        profile,
        data_dir,
        run_dir,
    });
    // A process that was killed has no exit code to give.
    ExitCode::from(end.exit_code().unwrap_or(USAGE))
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let _ = PathBuf::new();
    eprintln!("qsvmm runs on a QS/VS host only");
    ExitCode::from(USAGE)
}
