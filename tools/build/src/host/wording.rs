// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Two audits of the tree's text for `build doctor`:
//!
//! - [`names`]: no other vendor's product name or mark in any file git
//!   tracks (`CLAUDE.md`, ground rules): the screens, messages, help,
//!   documents and code alike. The rule's own line, and a protocol's
//!   token a peer requires, are the only allowances.
//! - [`help`]: every panel's help has a title and text (the `Panel`
//!   trait gives each one help; none may give it empty).

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

/// The names and marks that may not appear.
pub const FORBIDDEN: &[&str] = &[
    "IBM",
    "AS/400",
    "iSeries",
    "System i",
    "OS/400",
    "i5/OS",
    "PowerVM",
    "PowerHA",
    "Power Systems",
    "z/OS",
    "AIX",
    "DB2",
    "WebSphere",
];

/// Where one may: the rule itself, and the 5250 terminal type a
/// telnet server asks a display to name (RFC 4777), which the harness
/// must send to read QS/400's sign-on display.
const ALLOWED: &[(&str, &str)] = &[
    ("CLAUDE.md", "No IBM text, product names or marks anywhere"),
    ("", "IBM-3179-2"),
];

/// This audit's own source, which names what it looks for.
const SELF: &str = "tools/build/src/host/wording.rs";

/// Whether `text` holds `word` standing alone: not inside a longer word.
fn holds(text: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(at) = text[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        let before = text[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_word(c));
        let after = text[end..].chars().next().is_none_or(|c| !is_word(c));
        if before && after {
            return true;
        }
        from = end;
    }
    false
}

/// The forbidden names in one file's `text`, a line each.
pub fn names_in(file: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut checked = line.to_string();
        for (f, allowed) in ALLOWED {
            if f.is_empty() || file == *f {
                checked = checked.replace(allowed, "");
            }
        }
        for word in FORBIDDEN {
            if holds(&checked, word) {
                out.push(format!("{file}:{}: {word}", n + 1));
            }
        }
    }
    out
}

/// Every forbidden name in the files git tracks under `repo`; the
/// fuzzer's corpus (bytes, not text) is left out.
pub fn names(repo: &Path) -> Result<(usize, Vec<String>)> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(repo)
        .output()
        .context("git ls-files")?;
    let mut found = Vec::new();
    let mut count = 0;
    for file in String::from_utf8_lossy(&out.stdout).split('\0') {
        // The corpus is bytes; this file is the list itself.
        if file.is_empty() || file.starts_with("tools/fuzz/corpus/") || file == SELF {
            continue;
        }
        let Ok(bytes) = std::fs::read(repo.join(file)) else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        count += 1;
        found.extend(names_in(file, &text));
    }
    Ok((count, found))
}

/// Every panel's help with an empty title or no text, in the crates
/// that have panels.
pub fn help(repo: &Path) -> Result<(usize, Vec<String>)> {
    let mut found = Vec::new();
    let mut count = 0;
    for dir in ["base/panels/src", "base/install/src", "svc/tools/src"] {
        let mut stack = vec![repo.join(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).with_context(|| d.display().to_string())? {
                let p = e?.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&p)?;
                let rel = p.strip_prefix(repo).unwrap_or(&p).display().to_string();
                for (n, line) in text.lines().enumerate() {
                    let l = line.trim();
                    if l.starts_with("fn help(&self) -> Help") {
                        count += 1;
                    }
                    if l == "lines: &[]," || l == "title: \"\"," {
                        found.push(format!("{rel}:{}: empty help", n + 1));
                    }
                }
            }
        }
    }
    Ok((count, found))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_found_alone_and_not_inside_a_word() {
        assert_eq!(names_in("x.md", "runs AIX.\nmaiXed").len(), 1);
        assert!(names_in("x.md", "FAIXE and DB21").is_empty());
        assert_eq!(names_in("x.md", "an AS/400 and IBM").len(), 2);
        assert!(names_in("x.rs", "negotiated as IBM-3179-2").is_empty());
        assert_eq!(
            names_in("y.md", "No IBM text, product names or marks anywhere").len(),
            1
        );
        assert!(
            names_in(
                "CLAUDE.md",
                "- No IBM text, product names or marks anywhere."
            )
            .is_empty()
        );
    }

    #[test]
    fn the_tree_names_no_other_vendor_and_every_panel_has_help() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (files, found) = names(&repo).unwrap();
        assert!(files > 100);
        assert_eq!(found, Vec::<String>::new());
        let (panels, empty) = help(&repo).unwrap();
        assert!(panels > 10, "{panels}");
        assert_eq!(empty, Vec::<String>::new());
    }
}
