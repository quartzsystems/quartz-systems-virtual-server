// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's messages (`docs/MESSAGES.md`). A message has an identifier,
//! `QVS` and four digits; first-level text, one line that says what
//! happened; and second-level text, which says why and what to do.
//! `&1` to `&9` in either stand for the values a message is issued
//! with.
//!
//! - [`catalogue`]: every message.
//! - [`Message`] is a catalogue entry, [`Issued`] one with its values
//!   filled in, as it travels to whoever asked.

#![forbid(unsafe_code)]

pub mod catalogue;

pub use catalogue::*;

/// The prefix of every identifier.
pub const PREFIX: &str = "QVS";

/// A catalogue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    /// `QVS` and four digits.
    pub id: &'static str,
    /// First-level text.
    pub text: &'static str,
    /// Second-level text.
    pub help: &'static str,
}

/// A message with its values filled in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issued {
    /// The identifier.
    pub id: String,
    /// First-level text.
    pub text: String,
    /// Second-level text.
    pub help: String,
}
ipc::wire_struct!(Issued { id, text, help });

/// `text` with `&1` to `&9` replaced by `values`; a number with no
/// value stays as written.
fn fill(text: &str, values: &[&str]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let digit = chars.peek().and_then(|d| d.to_digit(10));
        match (c, digit) {
            ('&', Some(n)) if n >= 1 && values.len() >= n as usize => {
                out.push_str(values[n as usize - 1]);
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

impl Message {
    /// The message issued with `values` for `&1`, `&2` and so on.
    pub fn with(&self, values: &[&str]) -> Issued {
        Issued {
            id: self.id.to_string(),
            text: fill(self.text, values),
            help: fill(self.help, values),
        }
    }

    /// The message issued with no values.
    pub fn issue(&self) -> Issued {
        self.with(&[])
    }

    /// How many values the message takes: its highest `&n`.
    pub fn values(&self) -> usize {
        (1..=9usize)
            .rev()
            .find(|n| {
                let mark = format!("&{n}");
                self.text.contains(&mark) || self.help.contains(&mark)
            })
            .unwrap_or(0)
    }
}

impl Issued {
    /// Whether this is `message`, whatever its values.
    pub fn is(&self, message: &Message) -> bool {
        self.id == message.id
    }
}

impl std::fmt::Display for Issued {
    /// The identifier and the first-level text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.id, self.text)
    }
}

/// The catalogue as the table of `docs/MESSAGES.md`.
pub fn table() -> String {
    let mut rows = vec![
        "| Message | First-level text | Second-level text |".to_string(),
        "|---|---|---|".to_string(),
    ];
    rows.extend(
        ALL.iter()
            .map(|m| format!("| `{}` | {} | {} |", m.id, m.text, m.help)),
    );
    rows.join("\n") + "\n"
}

/// The catalogue entry with the identifier `id`.
pub fn find(id: &str) -> Option<&'static Message> {
    ALL.iter().copied().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_replace_their_numbers() {
        assert_eq!(fill("&1 of &2, &1", &["a", "b"]), "a of b, a");
        assert_eq!(fill("&3 & &0 &", &["a"]), "&3 & &0 &");
        let issued = NAME_INVALID.with(&["9LIVES"]);
        assert!(issued.text.contains("9LIVES"));
        assert!(issued.is(&NAME_INVALID));
        assert_eq!(
            issued.to_string(),
            format!("{} {}", NAME_INVALID.id, issued.text)
        );
        let back: Issued = ipc::decode(&ipc::encode(&issued)).unwrap();
        assert_eq!(back, issued);
    }

    #[test]
    fn every_message_has_its_own_identifier_and_both_texts() {
        for (i, m) in ALL.iter().enumerate() {
            assert!(
                m.id.len() == 7
                    && m.id.starts_with(PREFIX)
                    && m.id[3..].bytes().all(|b| b.is_ascii_digit()),
                "{}",
                m.id
            );
            assert!(ALL[..i].iter().all(|o| o.id != m.id), "{} twice", m.id);
            assert!(
                ALL[..i].iter().all(|o| o.id < m.id),
                "{} out of order",
                m.id
            );
            assert!(!m.text.is_empty() && !m.help.is_empty(), "{}", m.id);
            assert!(!m.text.contains('\n'), "{}", m.id);
            assert!(m.text.len() <= 132, "{} is {} long", m.id, m.text.len());
            // The values a message takes are numbered from 1, no gaps.
            for n in 1..=m.values() {
                let mark = format!("&{n}");
                assert!(
                    m.text.contains(&mark) || m.help.contains(&mark),
                    "{} has no {mark}",
                    m.id
                );
            }
            assert_eq!(find(m.id), Some(*m));
        }
        assert!(find("QVS0000").is_none());
    }

    /// With `QVS_MESSAGES=update` the document's table is written
    /// instead of compared.
    #[test]
    fn the_document_lists_every_message() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/MESSAGES.md");
        // A checkout may have given the document other line ends.
        let doc = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        let (begin, end) = ("<!-- messages -->\n", "<!-- end of messages -->");
        let table = crate::table();
        if std::env::var_os("QVS_MESSAGES").is_some_and(|v| v == "update") {
            let head = doc.split(begin).next().unwrap_or_default();
            let tail = doc.split(end).nth(1).unwrap_or("\n");
            std::fs::write(path, format!("{head}{begin}{table}{end}{tail}")).unwrap();
            return;
        }
        let listed = doc
            .split(begin)
            .nth(1)
            .and_then(|rest| rest.split(end).next())
            .unwrap_or_default();
        assert!(
            listed == table,
            "docs/MESSAGES.md is not the catalogue: run this test with QVS_MESSAGES=update"
        );
    }
}
