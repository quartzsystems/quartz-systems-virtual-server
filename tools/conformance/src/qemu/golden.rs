// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Golden files: every installer and Service Tools screen as text,
//! under `tools/conformance/golden/`. What changes from run to run or
//! from host to host is masked on both sides before the comparison:
//! the build ID, dates and times, a count of seconds, a process
//! number, and what the KVM probe found. `QSVS_GOLDEN=update` writes
//! the files instead of comparing; what it wrote is then read and
//! committed by a person.

use std::fs;
use std::path::PathBuf;

/// Set to `update` to write the golden files instead of comparing.
pub const ENV: &str = "QSVS_GOLDEN";

/// The words after which a number changes from run to run.
const COUNTED: [&str; 2] = ["process ", "sequence "];

/// The labels of rows whose value depends on the machine the suite
/// runs on.
const OF_THE_HOST: [&str; 1] = ["KVM"];

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

/// The length of `12.34 s` at the start of `bytes`: a count of seconds.
fn seconds(bytes: &[u8]) -> Option<usize> {
    let whole = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if whole == 0 || bytes.get(whole) != Some(&b'.') {
        return None;
    }
    let part = bytes[whole + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let end = whole + 1 + part;
    let unit = bytes[end..].starts_with(b" s")
        && !bytes.get(end + 2).is_some_and(u8::is_ascii_alphanumeric);
    (part > 0 && unit).then_some(end + 2)
}

/// One line with its dates, times and counts masked.
fn mask_line(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        let joined = |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b':');
        let before = i > 0 && joined(bytes.get(i - 1));
        // Not part of something longer: a hardware address, a number.
        let apart = |len: usize| !before && !joined(bytes.get(i + len));
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
        } else if let Some(len) = seconds(rest).filter(|_| !before) {
            out.push_str("#.## s");
            i += len;
        } else if let Some(word) = COUNTED.iter().find(|w| rest.starts_with(w.as_bytes())) {
            out.push_str(word);
            i += word.len();
            let run = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
            if run > 0 {
                out.push('#');
                i += run;
            }
        } else {
            // Screens are ASCII; anything else passes as it is.
            let c = line[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// `line` with the build ID after `release` masked: `V1R0M0 <build>`,
/// and `V1R0M0 build <build>` where a display spells it out.
fn mask_build(line: &str, release: &str) -> String {
    let mark = format!("{release} ");
    let Some(at) = line.find(&mark) else {
        return line.to_string();
    };
    let after = at + mark.len();
    let after = after
        + if line[after..].starts_with("build ") {
            6
        } else {
            0
        };
    let end = line[after..]
        .find(' ')
        .map_or(line.len(), |space| after + space);
    if end == after {
        return line.to_string();
    }
    format!("{}<build>{}", &line[..after], &line[end..])
}

/// `line` with its value masked, when its label is of the host.
fn mask_host(line: &str) -> String {
    let labelled = OF_THE_HOST.iter().any(|label| {
        line.trim_start()
            .strip_prefix(label)
            .is_some_and(|rest| rest.trim_start_matches([' ', '.']).starts_with(':'))
    });
    match line.find(':') {
        Some(colon) if labelled => format!("{} <of the host>", &line[..=colon]),
        _ => line.to_string(),
    }
}

/// A screen with what changes from run to run masked. `release` is the
/// release whose build ID follows it on a banner.
pub fn mask(screen: &str, release: &str) -> String {
    let mut out = String::new();
    for line in screen.lines() {
        let line = mask_host(&mask_build(line, release));
        out.push_str(mask_line(&line).trim_end());
        out.push('\n');
    }
    out
}

/// Hold `screen` against the golden file `name`, or write the file
/// when [`ENV`] says so.
pub fn check(name: &str, screen: &str, release: &str) -> Result<(), String> {
    let path = dir().join(format!("{name}.txt"));
    let masked = mask(screen, release);
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
    // A golden file checked out on another system may have other line ends.
    let golden = mask(&golden.replace('\r', ""), release);
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
        let screen = "QS/VS V1R0M0 phase2-12-gabc1234 (Quartz Systems/Virtual Server)\n\
                      Release  . . . . . : V1R0M0 build phase2-12-gabc1234 (test image)\n\
                      Date and time  2026-11-12 14:05 UTC+01:00\n\
                      The clock now shows 2026-11-12 14:05:59 UTC.\n\
                      Uptime . . . . . . : 0 days 00:03:12\n\
                      Reference code . . : C2001090 Base ready: 0.82 s after the kernel started\n\
                      KVM  . . . . . . . : KVM usable: VMX, API version 12\n\
                      logd       ready, process 87\n\
                      MAC 52:54:00:12:34:56 at 10.0.2.15/24, 671.0 MB, 30 s\n";
        let masked = mask(screen, "V1R0M0");
        assert_eq!(
            masked,
            "QS/VS V1R0M0 <build> (Quartz Systems/Virtual Server)\n\
             Release  . . . . . : V1R0M0 build <build> (test image)\n\
             Date and time  yyyy-mm-dd hh:mm UTC+hh:mm\n\
             The clock now shows yyyy-mm-dd hh:mm:ss UTC.\n\
             Uptime . . . . . . : 0 days hh:mm:ss\n\
             Reference code . . : C2001090 Base ready: #.## s after the kernel started\n\
             KVM  . . . . . . . : <of the host>\n\
             logd       ready, process #\n\
             MAC 52:54:00:12:34:56 at 10.0.2.15/24, 671.0 MB, 30 s\n"
        );
        // Masking what is masked changes nothing.
        assert_eq!(mask(&masked, "V1R0M0"), masked);
    }

    #[test]
    fn the_golden_directory_is_in_the_tree() {
        assert!(dir().ends_with("tools/conformance/golden"));
    }
}
