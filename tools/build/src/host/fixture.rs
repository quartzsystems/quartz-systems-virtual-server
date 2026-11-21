// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `build fixture`: the proof that another appliance can pin the base
//! crates. The fixture `QSDEMO` (`tools/fixture/`) is copied to a clean
//! directory, given a `Cargo.toml` that takes the base crates
//! ([`TAKES`]) as git dependencies at this repository's HEAD, built and
//! run. It passes when each of its parts
//! says it held, and its dependency graph has no crate of this
//! repository outside `base/`.
//!
//! Only committed work is seen: a git dependency is HEAD, not the
//! working tree.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use buildkit::Ctx;
use buildkit::host::cmd::Cmd;
use tracing::{info, warn};

/// The fixture in the tree.
const FIXTURE: &str = "tools/fixture";
/// What the fixture says, one line a part, when each held.
const PARTS: &[&str] = &[
    "processor",
    "qemu",
    "inventory",
    "netd",
    "update",
    "servicedump",
    "problem",
    "confine",
    "buildaid",
    "panics",
    "payload",
];
/// The base crates it takes.
const TAKES: &[&str] = &[
    "processor",
    "qemu",
    "inventory",
    "netd",
    "state",
    "msg",
    "problem",
    "servicedump",
    "confine",
    "update",
    "buildaid",
    "stream",
    "logd",
    "loadsource",
];

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(Cmd::new("git", "git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .capture()?
        .trim()
        .to_string())
}

/// The fixture in `dir`, its manifest naming this repository's HEAD:
/// the commit and the repository's URL.
fn lay_out(repo: &Path, dir: &Path) -> Result<(String, String)> {
    if dir.exists() {
        fs::remove_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    fs::create_dir_all(dir.join("src"))?;
    fs::copy(
        repo.join(FIXTURE).join("src/main.rs"),
        dir.join("src/main.rs"),
    )?;
    fs::copy(
        repo.join("rust-toolchain.toml"),
        dir.join("rust-toolchain.toml"),
    )?;
    let rev = git(repo, &["rev-parse", "HEAD"])?;
    if !git(repo, &["status", "--porcelain"])?.is_empty() {
        warn!("fixture: the working tree has changes; the fixture builds HEAD ({rev})");
    }
    let url = format!("file://{}", repo.display().to_string().replace('\\', "/"));
    let manifest = fs::read_to_string(repo.join(FIXTURE).join("Cargo.toml.in"))?
        .replace("@REPO@", &url)
        .replace("@REV@", &rev);
    fs::write(dir.join("Cargo.toml"), manifest)?;
    Ok((rev, url))
}

/// The package names of the repository's crates outside `base/`.
fn outside(repo: &Path) -> Result<Vec<String>> {
    let (_, problems) = super::neutral::check(repo, "qsvs")?;
    ensure!(problems.is_empty(), "base/ is not neutral: {problems:?}");
    super::neutral::outside_crates(repo)
}

/// `build fixture`. Returns one line for the report.
pub fn fixture(ctx: &Ctx) -> Result<String> {
    let repo = &ctx.paths.repo;
    let dir = ctx.paths.target.join("fixture");
    let (rev, url) = lay_out(repo, &dir)?;
    let target = dir.join("target");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    info!("fixture: building QSDEMO in {}", dir.display());
    let out = Cmd::new(&cargo, "fixture-run")
        .cwd(&dir)
        .args(["run", "--quiet"])
        .env("CARGO_TARGET_DIR", &target)
        .capture()?;
    for part in PARTS {
        let prefix = format!("QSDEMO {part}: ");
        ensure!(
            out.lines().any(|l| l.starts_with(&prefix)),
            "no `{prefix}` line from the fixture:\n{out}"
        );
    }

    // What it is built with: the base crates it takes, at HEAD, and
    // nothing else of this repository.
    let tree = Cmd::new(&cargo, "fixture-tree")
        .cwd(&dir)
        .args(["tree", "--locked", "-e", "normal,build"])
        .args(["--prefix", "none", "--format", "{p}"])
        .env("CARGO_TARGET_DIR", &target)
        .capture()?;
    let short = &rev[..rev.len().min(7)];
    let ours: Vec<&str> = tree
        .lines()
        .filter(|l| l.contains(url.as_str()))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    for take in TAKES {
        ensure!(
            ours.contains(take),
            "the fixture is not built with {take}:\n{tree}"
        );
    }
    ensure!(
        tree.lines()
            .filter(|l| l.contains(url.as_str()))
            .all(|l| l.contains(short)),
        "a crate of this repository is not at {short}:\n{tree}"
    );
    let outside = outside(repo)?;
    for name in &ours {
        ensure!(
            !outside.iter().any(|o| o == name),
            "the fixture is built with {name}, which is not a base crate"
        );
    }
    let mut taken: Vec<&str> = ours.clone();
    taken.sort_unstable();
    taken.dedup();
    let line = format!(
        "QSDEMO built from git dependencies at {short} with {} base crates ({}), none outside base/; {}",
        taken.len(),
        taken.join(", "),
        out.lines()
            .filter(|l| l.starts_with("QSDEMO "))
            .collect::<Vec<_>>()
            .join("; ")
    );
    info!("fixture: {line}");
    Ok(line)
}
