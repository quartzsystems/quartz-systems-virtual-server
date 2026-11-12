// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The license header check: every `.rs` file in the repository starts
//! with exactly the three header lines.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// What every source file starts with.
pub const HEADER: &str = "\
// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.
";

/// Directories that hold no sources of ours: build output and dot
/// directories (`.git`, `.cargo`, `.github`).
fn skipped(name: &str) -> bool {
    name == "target" || name.starts_with('.')
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| dir.display().to_string())? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if !skipped(&name) {
                walk(&path, files)?;
            }
        } else if name.ends_with(".rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Whether `text` starts with the header. A checkout with CRLF line
/// ends is the same file.
pub fn has_header(text: &str) -> bool {
    text.starts_with(HEADER) || text.replace("\r\n", "\n").starts_with(HEADER)
}

/// How many `.rs` files `repo` has, and the ones without the header,
/// relative to it.
pub fn check(repo: &Path) -> Result<(usize, Vec<String>)> {
    let mut files = Vec::new();
    walk(repo, &mut files)?;
    files.sort();
    let mut bad = Vec::new();
    for path in &files {
        let text = fs::read(path).with_context(|| path.display().to_string())?;
        if !has_header(&String::from_utf8_lossy(&text)) {
            let rel = path.strip_prefix(repo).unwrap_or(path);
            bad.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok((files.len(), bad))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_must_be_first_and_exact() {
        assert!(has_header(&format!("{HEADER}\nfn main() {{}}\n")));
        assert!(has_header(&HEADER.replace('\n', "\r\n")));
        assert!(!has_header(&format!("\n{HEADER}")));
        assert!(!has_header(&HEADER.replace("2026", "2025")));
        assert!(!has_header("fn main() {}\n"));
    }

    /// The check itself, over this repository: CI's header check.
    #[test]
    fn every_source_file_of_the_repository_has_the_header() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (count, bad) = check(&repo).unwrap();
        assert!(count > 0);
        assert!(bad.is_empty(), "no license header: {bad:?}");
    }
}
