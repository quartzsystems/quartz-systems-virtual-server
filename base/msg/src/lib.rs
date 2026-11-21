// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An appliance's messages: an identifier, first-level text that says
//! what happened, second-level text that says why and what to do. `&1`
//! to `&9` in either stand for the values a message is issued with.
//! The product owns its catalogue, its prefix and its words; a base
//! service is given the messages it refuses with.
//!
//! - [`Message`] is a catalogue entry, [`Issued`] one with its values
//!   filled in, as it travels to whoever asked.
//! - [`table`] is a catalogue as a document's table.

#![forbid(unsafe_code)]

/// A catalogue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    /// The product's prefix and four digits.
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

/// A catalogue as a document's table: identifier, first-level and
/// second-level text, a row each.
pub fn table(all: &[&Message]) -> String {
    let mut rows = vec![
        "| Message | First-level text | Second-level text |".to_string(),
        "|---|---|---|".to_string(),
    ];
    rows.extend(
        all.iter()
            .map(|m| format!("| `{}` | {} | {} |", m.id, m.text, m.help)),
    );
    rows.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: Message = Message {
        id: "XYZ0001",
        text: "Thing &1 is not &2.",
        help: "&1 again.",
    };

    #[test]
    fn values_replace_their_numbers() {
        assert_eq!(fill("&1 of &2, &1", &["a", "b"]), "a of b, a");
        assert_eq!(fill("&3 & &0 &", &["a"]), "&3 & &0 &");
        let issued = SAMPLE.with(&["A", "B"]);
        assert_eq!(issued.text, "Thing A is not B.");
        assert_eq!(issued.help, "A again.");
        assert!(issued.is(&SAMPLE));
        assert_eq!(SAMPLE.values(), 2);
        assert_eq!(issued.to_string(), "XYZ0001 Thing A is not B.");
        let back: Issued = ipc::decode(&ipc::encode(&issued)).unwrap();
        assert_eq!(back, issued);
        assert!(table(&[&SAMPLE]).contains("| `XYZ0001` |"));
    }
}
