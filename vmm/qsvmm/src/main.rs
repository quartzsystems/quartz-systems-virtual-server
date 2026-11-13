// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvmm [--config <file>] [--data <dir>] [--run <dir>]`: run the
//! partition of the configuration until it ends, and exit with the
//! code that says why (`qsvmm::end`). The configuration is read from
//! standard input, where the partition manager writes it, unless a
//! file is named.

use std::path::PathBuf;
use std::process::ExitCode;

/// The exit code of a command line that is not one.
const USAGE: u8 = 2;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    use qsvmm::run::{Options, run};

    let mut config = None;
    let mut usage = false;
    let mut data_dir = PathBuf::from(release::DATA_DIR);
    let mut run_dir = PathBuf::from(release::RUN_DIR);
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().map(PathBuf::from);
        match (arg.to_str(), value) {
            (Some("--config"), Some(value)) => config = Some(value),
            (Some("--data"), Some(value)) => data_dir = value,
            (Some("--run"), Some(value)) => run_dir = value,
            _ => usage = true,
        }
    }
    if usage {
        eprintln!("usage: qsvmm [--config <file>] [--data <dir>] [--run <dir>]");
        return ExitCode::from(USAGE);
    }
    let end = run(&Options {
        config,
        data_dir,
        run_dir,
    });
    // A process that was killed has no exit code to give.
    let code = end.exit_code().unwrap_or(USAGE);
    // The report's last line, for whoever could not wait for the
    // process.
    eprintln!("{} {code}", qsvmm::END_WORD);
    ExitCode::from(code)
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    let _ = PathBuf::new();
    eprintln!("qsvmm runs on a QS/VS host only");
    ExitCode::from(USAGE)
}
