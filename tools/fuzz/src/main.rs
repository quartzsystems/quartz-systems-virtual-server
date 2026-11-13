// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvsfuzz list`, and `qsvsfuzz run <target> [--secs N] [--seed N]
//! [--crashes DIR]`: fuzz one target and exit 0 when nothing was
//! found, 1 with the input written under the crashes directory when
//! something was. `cargo run -p build -- fuzz` runs them all.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use fuzz::engine::{self, Finding, SLOW, Target};
use fuzz::{cov, targets};

/// A run that has not come back after this long is a hang.
const HANG: Duration = Duration::from_secs(20);

fn save(crashes: &Path, target: &str, finding: &Finding) -> PathBuf {
    // A name from the input's own bytes: the same finding, the same file.
    let mut sum: u64 = 0xCBF2_9CE4_8422_2325;
    for byte in &finding.input {
        sum = (sum ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01B3);
    }
    let path = crashes.join(format!("{target}-{sum:016x}"));
    let _ = std::fs::create_dir_all(crashes);
    let _ = std::fs::write(&path, &finding.input);
    path
}

fn run(target: &'static Target, secs: u64, seed: u64, crashes: &Path) -> ExitCode {
    let since = Instant::now();
    engine::quiet_panics();
    // The watchdog: an input that never returns cannot be caught by
    // the loop it is stuck in.
    let (name, dir) = (target.name, crashes.to_path_buf());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(200));
            if let Some((input, ran)) = engine::stuck(since, HANG) {
                let finding = Finding {
                    input,
                    what: format!("still running after {ran:?}"),
                };
                let path = save(&dir, name, &finding);
                println!("fuzz {name}: FINDING {} ({})", finding.what, path.display());
                std::process::exit(1);
            }
        }
    });
    let report = engine::fuzz(target, seed, Duration::from_secs(secs), since);
    let guide = if cov::is_empty() {
        "no coverage counters".to_string()
    } else {
        format!("{} of {} edges", report.edges, cov::len())
    };
    match report.finding {
        None => {
            println!(
                "fuzz {}: {} runs in {secs} s, {} kept, {guide}, no finding",
                target.name, report.runs, report.kept
            );
            ExitCode::SUCCESS
        }
        Some(finding) => {
            let path = save(crashes, target.name, &finding);
            println!(
                "fuzz {}: FINDING after {} runs, seed {seed}: {} ({} bytes, {})",
                target.name,
                report.runs,
                finding.what,
                finding.input.len(),
                path.display()
            );
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || {
        eprintln!("usage: qsvsfuzz list | run <target> [--secs N] [--seed N] [--crashes DIR]");
        eprintln!("a run longer than {SLOW:?} is a finding");
        ExitCode::from(2)
    };
    match args.first().map(String::as_str) {
        Some("list") => {
            for target in targets::TARGETS {
                println!("{:<10} {}", target.name, target.about);
            }
            ExitCode::SUCCESS
        }
        Some("run") => {
            let Some(target) = args.get(1).and_then(|name| targets::find(name)) else {
                return usage();
            };
            let (mut secs, mut seed, mut crashes) = (60, 1, PathBuf::from("crashes"));
            let mut rest = args[2..].iter();
            while let Some(flag) = rest.next() {
                let value = rest.next();
                match (flag.as_str(), value) {
                    ("--secs", Some(v)) => secs = v.parse().unwrap_or(secs),
                    ("--seed", Some(v)) => seed = v.parse().unwrap_or(seed),
                    ("--crashes", Some(v)) => crashes = PathBuf::from(v),
                    _ => return usage(),
                }
            }
            run(target, secs, seed, &crashes)
        }
        _ => usage(),
    }
}
