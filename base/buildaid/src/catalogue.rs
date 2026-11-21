// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An appliance's catalogue as one file a console reads
//! (`catalogue.json`, `quartz.catalogue/1`): every reference code with
//! its meaning, its cause (its row of the product's code table) and its
//! recovery (the problem its condition opens and its message's
//! second-level text, or where a person sees it), and every message
//! with both its texts. [`audit`] is the same join, checked.

use msg::Message;
use problem::table::Rules;
use serde_json::{Value, json};

/// The catalogue's form.
pub const SCHEMA: &str = "quartz.catalogue/1";

/// One reference code of the product.
#[derive(Clone, Copy, Debug)]
pub struct Code {
    /// The code.
    pub code: &'static str,
    /// Its family, as the catalogue says it: `progress`, `attention`,
    /// `fatal`, `power-down`.
    pub family: &'static str,
    /// What it means: its text.
    pub meaning: &'static str,
    /// Whether it must have a row in the problem table: an attention or
    /// fatal code.
    pub needs_recovery: bool,
}

/// What the catalogue is made of: the product's.
#[derive(Clone, Copy, Debug)]
pub struct Tables<'a> {
    /// The product's name.
    pub product: &'a str,
    /// Every code.
    pub codes: &'a [Code],
    /// The codes' document: a table row a code, its first cell the code
    /// in backquotes, its third its cause.
    pub document: &'a str,
    /// The document's name, as a finding says it.
    pub document_name: &'a str,
    /// Whether a row's code is one of the product's (not one of
    /// another's that the document names).
    pub ours: fn(&str) -> bool,
    /// Its problem log's rules: a code's recovery.
    pub rules: &'a Rules,
    /// Every message.
    pub messages: &'a [&'a Message],
    /// The recovery of a code with no row in the problem table: its
    /// progress codes'.
    pub no_recovery: &'a str,
}

/// Each code's row of the document: its cause. A row's first cell is
/// the code in backquotes, eight characters, that `ours` takes.
#[must_use]
pub fn causes(document: &str, ours: fn(&str) -> bool) -> Vec<(String, String)> {
    document
        .lines()
        .filter_map(|l| {
            let cells: Vec<&str> = l.split('|').map(str::trim).collect();
            let code = cells.get(1)?.strip_prefix('`')?.strip_suffix('`')?;
            (code.len() == 8 && ours(code)).then(|| {
                (
                    code.to_string(),
                    cells.get(3).map(|c| (*c).to_string()).unwrap_or_default(),
                )
            })
        })
        .collect()
}

fn recovery(t: &Tables<'_>, code: &str) -> Value {
    match t.rules.condition(code) {
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
        None => json!({ "problem": false, "action": t.no_recovery }),
    }
}

/// The catalogue of `release`, build `build`, as JSON.
#[must_use]
pub fn json(t: &Tables<'_>, release: &str, build: &str) -> String {
    let causes = causes(t.document, t.ours);
    let codes: Vec<Value> = t
        .codes
        .iter()
        .map(|c| {
            let cause = causes
                .iter()
                .find(|(k, _)| k == c.code)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default();
            json!({
                "code": c.code,
                "family": c.family,
                "meaning": c.meaning,
                "cause": cause,
                "recovery": recovery(t, c.code),
            })
        })
        .collect();
    let messages: Vec<Value> = t
        .messages
        .iter()
        .map(|m| json!({ "id": m.id, "text": m.text, "help": m.help }))
        .collect();
    let all = json!({
        "schema": SCHEMA,
        "product": t.product,
        "release": release,
        "build": build,
        "codes": codes,
        "messages": messages,
    });
    let mut out = serde_json::to_string_pretty(&all).unwrap_or_default();
    out.push('\n');
    out
}

/// What is missing from the catalogue: a code without its row in the
/// document or with an empty cause, a row of no code, an attention or
/// fatal code outside the problem table, a message without both texts.
#[must_use]
pub fn audit(t: &Tables<'_>) -> Vec<String> {
    let causes = causes(t.document, t.ours);
    let doc = t.document_name;
    let mut out = Vec::new();
    for c in t.codes {
        match causes.iter().find(|(k, _)| k == c.code) {
            None => out.push(format!("{} has no row in {doc}", c.code)),
            Some((_, cause)) if cause.is_empty() => {
                out.push(format!("{} has no cause in {doc}", c.code));
            }
            Some(_) => {}
        }
        if c.needs_recovery && t.rules.condition(c.code).is_none() {
            out.push(format!(
                "{} is not in the problem table: no recovery",
                c.code
            ));
        }
    }
    for (k, _) in &causes {
        if !t.codes.iter().any(|c| c.code == k) {
            out.push(format!("{doc} has {k}, which no code is"));
        }
    }
    for m in t.messages {
        if m.text.trim().is_empty() || m.help.trim().is_empty() {
            out.push(format!("{} lacks a text", m.id));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "| Code | Text | Cause |\n|---|---|---|\n| `C1001000` | Started | It was powered on. |\n| `A1001010` | Unit failed | A disk went away. |\n";

    static NOTE: Message = Message {
        id: "XMP0001",
        text: "A thing.",
        help: "Do this.",
    };

    static RULES: Rules = Rules {
        conditions: &[problem::table::Condition {
            code: "A1001010",
            opens: None,
            note: "said on the display",
        }],
        severity: |_| problem::Severity::Attention,
        messages: problem::table::Messages {
            no_problem: &NOTE,
            closed: &NOTE,
            holds: &NOTE,
            not_kept: &NOTE,
            note_invalid: &NOTE,
        },
    };

    fn tables<'a>(codes: &'a [Code], messages: &'a [&'a Message]) -> Tables<'a> {
        Tables {
            product: "DEMO",
            codes,
            document: DOC,
            document_name: "docs/CODES.md",
            ours: |c| c.as_bytes()[1] == b'1',
            rules: &RULES,
            messages,
            no_recovery: "None: its progress.",
        }
    }

    const CODES: &[Code] = &[
        Code {
            code: "C1001000",
            family: "progress",
            meaning: "Started",
            needs_recovery: false,
        },
        Code {
            code: "A1001010",
            family: "attention",
            meaning: "Unit failed",
            needs_recovery: true,
        },
    ];

    #[test]
    fn a_whole_catalogue_has_every_code_and_message() {
        let messages: [&Message; 1] = [&NOTE];
        let t = tables(CODES, &messages);
        assert_eq!(audit(&t), Vec::<String>::new());
        let v: Value = serde_json::from_str(&json(&t, "V1R0M0", "b")).unwrap();
        assert_eq!(v["schema"], SCHEMA);
        assert_eq!(v["codes"][1]["cause"], "A disk went away.");
        assert_eq!(v["codes"][1]["recovery"]["action"], "said on the display");
        assert_eq!(v["messages"][0]["id"], "XMP0001");
    }

    #[test]
    fn what_is_missing_is_said() {
        let more = [
            CODES[0],
            CODES[1],
            Code {
                code: "A1001020",
                family: "attention",
                meaning: "Other",
                needs_recovery: true,
            },
        ];
        let empty = Message {
            id: "XMP0002",
            text: "",
            help: "x",
        };
        let messages: [&Message; 2] = [&NOTE, &empty];
        let found = audit(&tables(&more, &messages));
        assert!(
            found.iter().any(|f| f.contains("A1001020 has no row")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|f| f.contains("A1001020 is not in the problem table"))
        );
        assert!(found.iter().any(|f| f == "XMP0002 lacks a text"));
    }
}
