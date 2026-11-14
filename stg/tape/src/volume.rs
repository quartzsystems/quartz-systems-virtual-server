// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What is kept beside a volume's image: its identifier, its capacity
//! and whether it is write-protected. The image is `<name>.aws` in the
//! media repository's `tape` directory and this is `<name>.toml`.

use serde::{Deserialize, Serialize};

/// The most characters of a volume identifier.
pub const ID_MAX: usize = 6;
/// The least capacity, in MiB.
pub const MIN_CAPACITY_MIB: u64 = 16;
/// The most capacity, in MiB: 16 TiB.
pub const MAX_CAPACITY_MIB: u64 = 16 << 20;

/// A volume's description.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Volume {
    /// The schema of the file.
    pub schema_version: u32,
    /// The identifier: 1 to 6 of `A` to `Z` and `0` to `9`.
    pub identifier: String,
    /// The capacity in MiB.
    pub capacity_mib: u64,
    /// Whether it is write-protected.
    pub write_protected: bool,
    /// When it was made, in seconds since the epoch.
    pub created: u64,
}

/// Whether `id` is a volume identifier.
pub fn valid_identifier(id: &str) -> bool {
    (1..=ID_MAX).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

impl Volume {
    /// The capacity in bytes.
    pub fn capacity(&self) -> u64 {
        self.capacity_mib << 20
    }

    /// What is wrong with it, each thing once.
    pub fn problems(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !valid_identifier(&self.identifier) {
            out.push("identifier");
        }
        if !(MIN_CAPACITY_MIB..=MAX_CAPACITY_MIB).contains(&self.capacity_mib) {
            out.push("capacity");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_capacities_are_checked() {
        assert!(valid_identifier("VOL001"));
        assert!(valid_identifier("A"));
        assert!(!valid_identifier("VOL0001"));
        assert!(!valid_identifier("vol1"));
        assert!(!valid_identifier(""));
        let v = Volume {
            schema_version: 1,
            identifier: "SAV01".into(),
            capacity_mib: 1024,
            write_protected: false,
            created: 0,
        };
        assert!(v.problems().is_empty());
        assert_eq!(v.capacity(), 1 << 30);
        let bad = Volume {
            identifier: "x".into(),
            capacity_mib: 1,
            ..v
        };
        assert_eq!(bad.problems(), ["identifier", "capacity"]);
    }
}
