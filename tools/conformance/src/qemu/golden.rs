// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Golden files: every installer and Service Tools screen as text,
//! under `tools/conformance/golden/`. What changes from run to run is
//! masked on both sides before the comparison: the build ID, dates
//! and times, and the numbers after a few labels (an uptime, a
//! process). `QSVS_GOLDEN=update` writes the files instead of
//! comparing; what it wrote is then read and committed by a person.

use std::fs;
use std::path::PathBuf;

/// Set to `update` to write the golden files instead of comparing.
pub const ENV: &str = "QSVS_GOLDEN";

/// The words after which a number changes from run to run.
const COUNTED: [&str; 2] = ["process ", "sequence "];

/// The directory of the golden files.
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden")
}

fn digits(bytes: &[u8], lens: &[usize], seps: &[u8]) -> bool {
    let mut at = 0;
    for (i, len) in lens.iter().enumerate() {
        if bytes.len() < at + len || !bytes[at..at + len].iter().all(u8::is_ascii_digit) {
            return false;
        }
        at += len;
        if i < seps.len() {
            if bytes.get(at) != Some(&seps[i]) {
                return false;
            }
            at += 1;
        }
    }
    true
}

/// One line with its dates, times and counted numbers masked.
fn mask_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        // Not part of something longer: a hardware address, a number.
        let apart = |len: usize| {
            let joined =
                |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b':');
            let before = i > 0 && joined(bytes.get(i - 1));
            !before && !joined(bytes.get(i + len))
        };
        // 2026-11-12, then 14:05:30 or 14:05.
        if digits(rest, &[4, 2, 2], b"--") && apart(10) {
            out.push_str("yyyy-mm-dd");
            i += 10;
        } else if digits(rest, &[2, 2, 2], b"::") && apart(8) {
            out.push_str("hh:mm:ss");
            i += 8;
        } else if digits(rest, &[2, 2], b":") && apart(5) {
            out.push_str("hh:mm");
            i += 5;
        } else if let Some(word) = COUNTED.iter().find(|w| rest.starts_with(w.as_bytes())) {
            out.push_str(word);
            i += word.len();
            let run = bytes[i..]
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b'.' || **b == b' ' || **b == b':')
                .count();
            if bytes[i..i + run].iter().any(u8::is_ascii_digit) {
                out.push('#');
                // What follows keeps its distance from the label.
                i += run;
                if bytes.get(i - 1) == Some(&b' ') {
                    out.push(' ');
                }
            }
        } else {
            // Screens are ASCII; anything else passes as it is.
            let c = line[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out.trim_end().to_string()
}

/// A screen with what changes from run to run masked.
pub fn mask(screen: &str, build_id: &str) -> String {
    let mut out = String::new();
    for line in screen.lines() {
        let line = if build_id.is_empty() {
            line.to_string()
        } else {
            line.replace(build_id, "<build>")
        };
        out.push_str(&mask_line(&line));
        out.push('\n');
    }
    out
}

/// Hold `screen` against the golden file `name`, or write the file
/// when [`ENV`] says so.
pub fn check(name: &str, screen: &str, build_id: &str) -> Result<(), String> {
    let path = dir().join(format!("{name}.txt"));
    let masked = mask(screen, build_id);
    if std::env::var(ENV).is_ok_and(|v| v == "update") {
        fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
        return fs::write(&path, masked).map_err(|e| format!("{}: {e}", path.display()));
    }
    let golden = fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e} (run with {ENV}=update to write it)",
            path.display()
        )
    })?;
    // A golden file edited on another system may have other line ends.
    let golden = mask(&golden.replace('\r', ""), build_id);
    if golden == masked {
        return Ok(());
    }
    let differ = golden
        .lines()
        .zip(masked.lines())
        .position(|(a, b)| a != b)
        .map_or(0, |i| i + 1);
    Err(format!(
        "screen `{name}` differs from its golden file at row {differ}:\n--- golden\n{golden}--- seen\n{masked}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_changes_from_run_to_run_is_masked() {
        let screen = "QS/VS V1R0M0 phase2-12-gabc1234\n\
                      Date and time  2026-11-12 14:05 UTC+01:00\n\
                      The clock now shows 2026-11-12 14:05:59 UTC.\n\
                      Uptime . . . . :  0 days 00:03:12\n\
                      logd       ready, process 87\n\
                      MAC 52:54:00:12:34:56 at 10.0.2.15/24\n";
        let masked = mask(screen, "phase2-12-gabc1234");
        assert_eq!(
            masked,
            "QS/VS V1R0M0 <build>\n\
             Date and time  yyyy-mm-dd hh:mm UTC+hh:mm\n\
             The clock now shows yyyy-mm-dd hh:mm:ss UTC.\n\
             Uptime . . . . :  0 days hh:mm:ss\n\
             logd       ready, process #\n\
             MAC 52:54:00:12:34:56 at 10.0.2.15/24\n"
        );
        // Masking what is masked changes nothing.
        assert_eq!(mask(&masked, "phase2-12-gabc1234"), masked);
    }

    #[test]
    fn the_golden_directory_is_in_the_tree() {
        assert!(dir().ends_with("tools/conformance/golden"));
    }
}
