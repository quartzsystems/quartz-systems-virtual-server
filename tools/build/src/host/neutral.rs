// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The neutrality check of `base/`. Those crates are what every Quartz
//! appliance needs; QS/MC pins them from this repository. So their
//! code names no product: everything of QS/VS's comes in through the
//! `Appliance` description. The check fails on the product's
//! identifier in a `base/` source file outside its tests and comments,
//! and on a `base/` crate that depends on a crate of `lic/`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Where the neutral crates are.
const BASE: &str = "base";
/// Where the product's own crates are.
const LIC: &str = "lic";

/// The lines of a source file that are code and not tests: comment
/// lines are left out, and everything from the tests module at the
/// file's end (`#[cfg(test)]`, then `mod tests {`).
fn code(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let tests = lines
        .windows(2)
        .position(|w| w[0].trim() == "#[cfg(test)]" && w[1].trim() == "mod tests {")
        .unwrap_or(lines.len());
    lines
        .into_iter()
        .enumerate()
        .take(tests)
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
}

/// The lines of `text` that name the product `id`, in either case.
fn names_product(text: &str, id: &str) -> Vec<usize> {
    let id = id.to_ascii_lowercase();
    code(text)
        .filter(|(_, l)| l.to_ascii_lowercase().contains(&id))
        .map(|(n, _)| n + 1)
        .collect()
}

/// The dependencies of a `Cargo.toml` that are path dependencies into
/// `lic/`, or crates of that name.
fn lic_dependencies(manifest: &str, lic: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_dependencies = line.contains("dependencies");
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim().trim_end_matches(".workspace");
        let into_lic = value.contains(&format!("/{LIC}/")) || value.contains(&format!("\"{LIC}/"));
        if in_dependencies && (into_lic || lic.iter().any(|l| l == name)) {
            found.push(name.to_string());
        }
    }
    found
}

fn sources(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| dir.display().to_string())? {
        let path = entry?.path();
        if path.is_dir() {
            sources(&path, files)?;
        } else if path.extension().is_some_and(|x| x == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn subdirectories(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| dir.display().to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Cargo.toml").is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// How many `base/` crates `repo` has, and what is not neutral in them.
pub fn check(repo: &Path, id: &str) -> Result<(usize, Vec<String>)> {
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    let lic: Vec<String> = subdirectories(&repo.join(LIC))?
        .iter()
        .map(|p| name(p))
        .collect();
    let crates = subdirectories(&repo.join(BASE))?;
    let mut problems = Vec::new();
    for dir in &crates {
        let manifest = fs::read_to_string(dir.join("Cargo.toml"))?;
        for dependency in lic_dependencies(&manifest, &lic) {
            problems.push(format!(
                "{BASE}/{} depends on {LIC}/{dependency}",
                name(dir)
            ));
        }
        let mut files = Vec::new();
        sources(dir, &mut files)?;
        files.sort();
        for path in files {
            // A file of tests alone.
            if name(&path) == "tests.rs" {
                continue;
            }
            let text = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
            let rel = path.strip_prefix(repo).unwrap_or(&path);
            for line in names_product(&text, id) {
                problems.push(format!(
                    "{}:{line} names {id}",
                    rel.to_string_lossy().replace('\\', "/")
                ));
            }
        }
    }
    Ok((crates.len(), problems))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_what_is_before_the_tests_and_no_comment() {
        let text = "//! qsvs in a doc\nconst A: &str = \"x\"; // qsvs after code counts\n\
                    #[cfg(test)]\nmod tests;\nlet b = \"QSVS_\";\n    // qsvs in a comment\n\
                    #[cfg(test)]\nmod tests {\n    \"qsvs\"\n}\n";
        assert_eq!(names_product(text, "qsvs"), [2, 5]);
        assert!(names_product("fn appliance() {}\n", "qsvs").is_empty());
    }

    #[test]
    fn a_dependency_into_lic_is_found_by_path_or_by_name() {
        let lic = vec!["release".to_string(), "refcode".to_string()];
        let manifest = "[package]\nname = \"state\"\ndescription = \"no release here\"\n\n\
                        [dependencies]\nappliance = { path = \"../appliance\" }\n\
                        serde.workspace = true\nrefcode.workspace = true\n\
                        other = { path = \"../../lic/hostdisk\" }\n\n\
                        [target.'cfg(unix)'.dependencies]\nrelease = { version = \"1\" }\n";
        assert_eq!(
            lic_dependencies(manifest, &lic),
            ["refcode", "other", "release"]
        );
        assert!(lic_dependencies("[dependencies]\nipc.workspace = true\n", &lic).is_empty());
    }

    /// The check itself, over this repository: acceptance 4 of Phase 1.
    #[test]
    fn base_is_product_neutral() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (count, problems) = check(&repo, "qsvs").unwrap();
        assert!(count >= 6, "{count} base crates");
        assert!(problems.is_empty(), "base/ is not neutral: {problems:#?}");
    }
}
