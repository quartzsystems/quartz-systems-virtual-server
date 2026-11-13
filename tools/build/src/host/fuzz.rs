// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build fuzz`: the fuzzer (`tools/fuzz`) built with LLVM's coverage
//! counters, and every target run for a time. A target that finds
//! something fails the step; its input is under
//! `<target>/fuzz/crashes/`.

use std::env;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail, ensure};
use buildkit::Ctx;
use buildkit::host::cmd::Cmd;
use tracing::{error, info};

/// The flags that make the code count its edges: `SanitizerCoverage`
/// with inline 8-bit counters, which the fuzzer reads.
const SANCOV: &str = "-C passes=sancov-module -C llvm-args=-sanitizer-coverage-level=3 \
                      -C llvm-args=-sanitizer-coverage-inline-8bit-counters";
/// The host triple the fuzzer is built for. Naming it keeps the flags
/// off build scripts and procedural macros.
const HOST: &str = "x86_64-unknown-linux-gnu";
/// The fuzzer's package and its binary.
const PACKAGE: &str = "fuzz";
const BINARY: &str = "qsvsfuzz";
/// The cargo profile: optimized, a panic unwinds, arithmetic is checked.
const PROFILE: &str = "fuzz";

/// Where the fuzzer is built and keeps what it finds.
fn dir(ctx: &Ctx) -> PathBuf {
    ctx.paths.target.join("fuzz")
}

/// Build the fuzzer with the counters. The binary.
fn build(ctx: &Ctx) -> Result<PathBuf> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    info!(
        "fuzz: cargo build --profile {PROFILE} -p {PACKAGE} --target {HOST}, with coverage counters"
    );
    let mut cmd = Cmd::new(cargo, "fuzz");
    cmd.cwd(&ctx.paths.repo)
        .env("RUSTFLAGS", SANCOV)
        .args(["build", "--locked", "--profile", PROFILE, "-p", PACKAGE])
        .args(["--target", HOST])
        .arg("--target-dir")
        .arg(dir(ctx))
        .run(ctx)?;
    let binary = dir(ctx).join(HOST).join(PROFILE).join(BINARY);
    ensure!(binary.is_file(), "{} was not built", binary.display());
    Ok(binary)
}

/// The targets the fuzzer has.
fn targets(binary: &PathBuf) -> Result<Vec<String>> {
    let out = Command::new(binary)
        .arg("list")
        .output()
        .with_context(|| format!("cannot run {}", binary.display()))?;
    ensure!(out.status.success(), "`{BINARY} list` failed");
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_ascii_whitespace().next())
        .map(str::to_string)
        .collect())
}

/// `build fuzz`: every target, or the one named, for `secs` seconds
/// each, `jobs` at a time.
pub fn fuzz(ctx: &Ctx, only: Option<&str>, secs: u64, jobs: usize) -> Result<()> {
    let binary = build(ctx)?;
    let mut names = targets(&binary)?;
    if let Some(only) = only {
        ensure!(
            names.iter().any(|n| n == only),
            "no fuzz target `{only}`; the targets are {}",
            names.join(", ")
        );
        names.retain(|n| n == only);
    }
    let crashes = dir(ctx).join("crashes");
    info!(
        "fuzz: {} target(s), {secs} s each, {} at a time",
        names.len(),
        jobs.max(1)
    );
    let mut findings = 0;
    for batch in names.chunks(jobs.max(1)) {
        let running: Vec<(&String, Child)> = batch
            .iter()
            .map(|name| {
                Command::new(&binary)
                    .args(["run", name, "--secs", &secs.to_string(), "--crashes"])
                    .arg(&crashes)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map(|child| (name, child))
                    .with_context(|| format!("cannot start the fuzzer for {name}"))
            })
            .collect::<Result<_>>()?;
        for (name, child) in running {
            let out = child.wait_with_output()?;
            let said = String::from_utf8_lossy(&out.stdout);
            let line = said.lines().last().unwrap_or("said nothing");
            if out.status.success() {
                info!("{line}");
            } else {
                findings += 1;
                error!("{line}");
                let errors = String::from_utf8_lossy(&out.stderr);
                if !errors.trim().is_empty() {
                    error!("fuzz {name}: {}", errors.trim());
                }
            }
        }
    }
    if findings > 0 {
        bail!(
            "fuzz: {findings} target(s) found something; the inputs are under {}",
            crashes.display()
        );
    }
    info!("fuzz: PASS, {} target(s), no finding", names.len());
    Ok(())
}
