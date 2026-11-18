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
//! without a cause or a recovery, or a message without both texts.

use std::fmt::Write as _;

use serde_json::{Value, json};

/// The catalogue's form.
pub const SCHEMA: &str = "quartz.catalogue/1";

/// The reference codes' table, as the build reads it.
const REFCODES: &str = include_str!("../../../../docs/REFCODES.md");

/// Each code's row of `docs/REFCODES.md`: its cause.
fn causes() -> Vec<(String, String)> {
    REFCODES
        .lines()
        .filter_map(|l| {
            let cells: Vec<&str> = l.split('|').map(str::trim).collect();
            // | `C2001000` | text | cause |
            let code = cells.get(1)?.strip_prefix('`')?.strip_suffix('`')?;
            (code.len() == 8 && code.as_bytes()[1] == b'2').then(|| {
                (
                    code.to_string(),
                    cells.get(3).map(|c| (*c).to_string()).unwrap_or_default(),
                )
            })
        })
        .collect()
}

fn family(f: refcode::Family) -> &'static str {
    match f {
        refcode::Family::Progress => "progress",
        refcode::Family::Attention => "attention",
        refcode::Family::Fatal => "fatal",
        refcode::Family::PowerDown => "power-down",
    }
}

fn recovery(code: &refcode::RefCode) -> Value {
    match qvsmsg::problems::condition(code.code) {
        Some(c) => match &c.opens {
            Some(o) => json!({
                "problem": true,
                "message": o.message.id,
                "description": o.message.text,
                "action": o.message.help,
                "resources": o.resources,
                "cleared_by": o.cleared_by,
            }),
            None => json!({ "problem": false, "action": c.note }),
        },
        None => json!({ "problem": false, "action": "None: the host's progress." }),
    }
}

/// The catalogue of `release`, build `build`, as JSON.
pub fn json(release: &str, build: &str) -> String {
    let causes = causes();
    let codes: Vec<Value> = refcode::ALL
        .iter()
        .map(|c| {
            let cause = causes
                .iter()
                .find(|(k, _)| k == c.code)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default();
            json!({
                "code": c.code,
                "family": family(c.family),
                "meaning": c.text,
                "cause": cause,
                "recovery": recovery(c),
            })
        })
        .collect();
    let messages: Vec<Value> = qvsmsg::ALL
        .iter()
        .map(|m| json!({ "id": m.id, "text": m.text, "help": m.help }))
        .collect();
    let all = json!({
        "schema": SCHEMA,
        "product": release::NAME,
        "release": release,
        "build": build,
        "codes": codes,
        "messages": messages,
    });
    let mut out = serde_json::to_string_pretty(&all).unwrap_or_default();
    out.push('\n');
    out
}

/// What is missing from the catalogue: a code without its row in
/// `docs/REFCODES.md` or with an empty cause, a row of no code, an
/// attention or fatal code outside the problem table, a message without
/// both texts.
pub fn audit() -> Vec<String> {
    let causes = causes();
    let mut out = Vec::new();
    for c in refcode::ALL {
        match causes.iter().find(|(k, _)| k == c.code) {
            None => out.push(format!("{} has no row in docs/REFCODES.md", c.code)),
            Some((_, cause)) if cause.is_empty() => {
                out.push(format!("{} has no cause in docs/REFCODES.md", c.code));
            }
            Some(_) => {}
        }
        let needs = matches!(
            c.family,
            refcode::Family::Attention | refcode::Family::Fatal
        );
        if needs && qvsmsg::problems::condition(c.code).is_none() {
            out.push(format!(
                "{} is not in the problem table: no recovery",
                c.code
            ));
        }
    }
    for (k, _) in &causes {
        if !refcode::ALL.iter().any(|c| c.code == k) {
            out.push(format!("docs/REFCODES.md has {k}, which no code is"));
        }
    }
    for m in qvsmsg::ALL {
        if m.text.trim().is_empty() || m.help.trim().is_empty() {
            out.push(format!("{} lacks a text", m.id));
        }
    }
    out
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
