// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `identity.toml`: what the host worked out about itself and keeps,
//! apart from what an operator configures.

use serde::{Deserialize, Serialize};

use crate::store::{Document, Migration};

/// `identity.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// The system serial number derived when DMI has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
}

impl Document for Identity {
    const FILE: &'static str = "identity.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        if self.serial.as_deref().is_some_and(str::is_empty) {
            return Err("the serial number is empty".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{parse, render};

    #[test]
    fn it_round_trips_and_an_empty_file_is_the_defaults() {
        let empty: Identity = parse("schema_version = 1\n").unwrap();
        assert_eq!(empty, Identity::default());
        let kept = Identity {
            serial: Some("M525400123456".into()),
        };
        assert_eq!(parse::<Identity>(&render(&kept).unwrap()).unwrap(), kept);
    }

    #[test]
    fn bad_values_are_refused() {
        for text in ["serial = \"\"", "name = \"x\""] {
            let text = format!("schema_version = 1\n{text}\n");
            assert!(parse::<Identity>(&text).is_err(), "{text}");
        }
    }
}
