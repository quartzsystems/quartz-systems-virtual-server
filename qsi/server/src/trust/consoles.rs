// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The consoles paired with this host: `qsi-consoles.toml`, written
//! through the state store. A console is its certificate's fingerprint
//! and the name it gave when it paired.

use serde::{Deserialize, Serialize};
use state::{Document, Migration};

/// The most consoles paired at once: one, and a second.
pub const MAX_CONSOLES: usize = 2;

/// The longest console name kept.
const MAX_NAME: usize = 64;

/// One paired console.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Console {
    /// The name it gave when it paired.
    pub name: String,
    /// Its certificate's SHA-256, 64 hexadecimal digits.
    pub fingerprint: String,
    /// When it paired, in seconds since the epoch.
    pub paired: u64,
    /// When it last had a session, in seconds since the epoch; 0 for
    /// never.
    #[serde(default)]
    pub last_contact: u64,
    /// The address it last connected from; empty for none yet.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub address: String,
}

/// Every paired console.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Consoles {
    /// The consoles, in the order they paired.
    #[serde(default, rename = "console", skip_serializing_if = "Vec::is_empty")]
    pub consoles: Vec<Console>,
}

impl Document for Consoles {
    const FILE: &'static str = "qsi-consoles.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        if self.consoles.len() > MAX_CONSOLES {
            return Err(format!(
                "{} consoles; at most {MAX_CONSOLES}",
                self.consoles.len()
            ));
        }
        for c in &self.consoles {
            if qsitrust::Fingerprint::from_hex(&c.fingerprint).is_none() {
                return Err(format!("console {}: not a fingerprint", c.name));
            }
            if c.name.len() > MAX_NAME {
                return Err(format!("a console's name is over {MAX_NAME} bytes"));
            }
        }
        let mut prints: Vec<&str> = self
            .consoles
            .iter()
            .map(|c| c.fingerprint.as_str())
            .collect();
        prints.sort_unstable();
        prints.dedup();
        if prints.len() != self.consoles.len() {
            return Err("a console is paired twice".into());
        }
        Ok(())
    }
}

impl Consoles {
    /// The console with this certificate.
    #[must_use]
    pub fn by_print(&self, fingerprint: &qsitrust::Fingerprint) -> Option<&Console> {
        let hex = fingerprint.hex();
        self.consoles.iter().find(|c| c.fingerprint == hex)
    }

    /// Whether another console may pair.
    #[must_use]
    pub fn room(&self) -> bool {
        self.consoles.len() < MAX_CONSOLES
    }

    /// A console's name as kept: bounded, with no control characters.
    #[must_use]
    pub fn kept_name(name: &str) -> String {
        let clean: String = name.chars().filter(|c| !c.is_control()).collect();
        let mut end = clean.len().min(MAX_NAME);
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean[..end].to_string()
    }
}
