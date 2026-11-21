// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The catalogue as one file a console reads (`catalogue.json` in a
//! release's directory, its form in `docs/catalogue.schema.json`): every
//! reference code with its meaning (`refcode`), its cause
//! (`docs/REFCODES.md`'s row) and its recovery (the problem it opens and
//! its message's second-level text, or where a person sees it), and
//! every `QVS` message with both its texts. QS/MC shows a code's
//! explanation and guided repair from it without restating either.
//!
//! [`audit`] is the same join, checked: `build doctor` fails on a code
//! without a cause or a recovery, or a message without both texts. The
//! join is `base/buildaid`'s; the tables are QS/VS's.

use std::fmt::Write as _;
use std::sync::LazyLock;

use buildaid::catalogue::{Code, Tables};
#[cfg(test)]
use serde_json::Value;

/// The catalogue's form.
#[cfg(test)]
pub const SCHEMA: &str = buildaid::catalogue::SCHEMA;

/// The reference codes' table, as the build reads it.
const REFCODES: &str = include_str!("../../../../docs/REFCODES.md");

fn family(f: refcode::Family) -> &'static str {
    match f {
        refcode::Family::Progress => "progress",
        refcode::Family::Attention => "attention",
        refcode::Family::Fatal => "fatal",
        refcode::Family::PowerDown => "power-down",
    }
}

/// Every code of the host, as the catalogue has it.
static CODES: LazyLock<Vec<Code>> = LazyLock::new(|| {
    refcode::ALL
        .iter()
        .map(|c| Code {
            code: c.code,
            family: family(c.family),
            meaning: c.text,
            needs_recovery: matches!(
                c.family,
                refcode::Family::Attention | refcode::Family::Fatal
            ),
        })
        .collect()
});

/// QS/VS's tables: its codes, `docs/REFCODES.md` (a host code's second
/// character is `2`), its problem table, its messages.
fn tables() -> Tables<'static> {
    Tables {
        product: release::NAME,
        codes: &CODES,
        document: REFCODES,
        document_name: "docs/REFCODES.md",
        ours: |c| c.as_bytes().get(1) == Some(&b'2'),
        rules: &qvsmsg::problems::RULES,
        messages: qvsmsg::ALL,
        no_recovery: "None: the host's progress.",
    }
}

/// The catalogue of `release`, build `build`, as JSON.
pub fn json(release: &str, build: &str) -> String {
    buildaid::catalogue::json(&tables(), release, build)
}

/// What is missing from the catalogue (`buildaid::catalogue::audit`).
pub fn audit() -> Vec<String> {
    buildaid::catalogue::audit(&tables())
}

/// The audit as one line for `doctor`.
pub fn summary() -> Result<String, String> {
    let missing = audit();
    if missing.is_empty() {
        let mut s = String::new();
        let _ = write!(
            s,
            "{} codes with meaning, cause and recovery; {} messages with both texts",
            refcode::ALL.len(),
            qvsmsg::ALL.len()
        );
        Ok(s)
    } else {
        Err(missing.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/catalogue.json` is the catalogue of this tree's tables,
    /// built `source`: what a console builds into its image at the
    /// commit it pins, without QS/VS's release build. With
    /// `QVS_CATALOGUE=update` it is written instead of compared.
    #[test]
    fn the_committed_catalogue_is_the_tables() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/catalogue.json");
        let want = json(&release::RELEASE.to_string(), "source");
        if std::env::var_os("QVS_CATALOGUE").is_some_and(|v| v == "update") {
            std::fs::write(path, &want).unwrap();
            return;
        }
        let kept = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            kept == want,
            "docs/catalogue.json is not the tables': run this test with QVS_CATALOGUE=update"
        );
    }

    #[test]
    fn every_code_and_message_is_whole() {
        assert_eq!(audit(), Vec::<String>::new());
    }

    #[test]
    fn the_catalogue_has_every_code_and_message() {
        let v: Value = serde_json::from_str(&json("V1R0M0", "test")).unwrap();
        assert_eq!(v["schema"], SCHEMA);
        assert_eq!(v["codes"].as_array().unwrap().len(), refcode::ALL.len());
        assert_eq!(v["messages"].as_array().unwrap().len(), qvsmsg::ALL.len());
        let held = v["codes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["code"] == "A2005170")
            .unwrap();
        assert_eq!(held["recovery"]["problem"], true);
        assert!(!held["cause"].as_str().unwrap().is_empty());
    }
}
