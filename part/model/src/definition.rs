// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's definition: what the partition is, whatever profile
//! it is activated with.

use serde::{Deserialize, Serialize};
use state::store::{Document, Migration};

/// The highest partition ID. A partition's `qsvmm` runs as user and
/// group [`USER_BASE`] plus its ID, and the last two characters of its
/// serial number are its ID.
pub const MAX_ID: u32 = 999;
/// A partition's user and group are this plus its ID.
pub const USER_BASE: u32 = 23000;
/// The characters of a serial number: as QS/400's `QSRLNBR` holds it.
pub const SERIAL_LEN: usize = 7;
/// The longest auto start delay, in seconds: an hour.
pub const MAX_DELAY_SECS: u32 = 3600;

/// The user and group of the partition `id`'s `qsvmm`.
pub fn user_of(id: u32) -> u32 {
    USER_BASE + id
}

/// What runs in the partition.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Environment {
    /// QS/400. The only one.
    #[default]
    #[serde(rename = "QS/400")]
    Qs400,
}
ipc::wire_enum!(Environment { 0 => Qs400 });

impl Environment {
    /// As it is shown.
    pub fn word(self) -> &'static str {
        "QS/400"
    }
}

/// The keylock position. It is the host's: written to the partition's
/// LIC record at every activation.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Keylock {
    /// Unattended.
    #[default]
    Normal,
    /// Attended: the IPL stops at its menu.
    Manual,
    /// Unattended, and remote and timed IPLs are allowed.
    Auto,
    /// Unattended, and the panel is locked.
    Secure,
}
ipc::wire_enum!(Keylock { 0 => Normal, 1 => Manual, 2 => Auto, 3 => Secure });

impl Keylock {
    /// As a definition has it.
    pub fn word(self) -> &'static str {
        match self {
            Keylock::Normal => "normal",
            Keylock::Manual => "manual",
            Keylock::Auto => "auto",
            Keylock::Secure => "secure",
        }
    }
}

/// `[auto_start]`: whether the partition is activated when the host
/// has started, and when among the others.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AutoStart {
    /// Whether it is.
    #[serde(default)]
    pub enabled: bool,
    /// Its place: lower first; partitions with one place go by ID.
    #[serde(default)]
    pub order: u32,
    /// Seconds to wait before it, after the one before.
    #[serde(default)]
    pub delay_secs: u32,
}
ipc::wire_struct!(AutoStart {
    enabled,
    order,
    delay_secs
});

/// A partition's definition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    /// Its ID: assigned when it is created, and no other partition's
    /// while the definition exists.
    pub id: u32,
    /// Its name: no other partition's.
    pub name: String,
    /// What runs in it.
    #[serde(default)]
    pub environment: Environment,
    /// Its system serial number: made when it is created, never
    /// changed.
    pub serial: String,
    /// The keylock position.
    #[serde(default)]
    pub keylock: Keylock,
    /// The profile an activation uses when it names none; empty while
    /// the partition has no profile.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_profile: String,
    /// Auto start.
    #[serde(default)]
    pub auto_start: AutoStart,
    /// When it was created, in seconds since the epoch.
    pub created: u64,
}
ipc::wire_struct!(Definition {
    id,
    name,
    environment,
    serial,
    keylock,
    default_profile,
    auto_start,
    created
});

impl Document for Definition {
    const FILE: &'static str = "definition.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    /// What a definition may hold is [`crate::validate`]'s.
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// The serial number's alphabet: digits and letters, without `I` and
/// `O`.
const ALPHABET: &[u8; 34] = b"0123456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// The serial number of the partition `id` of the host whose serial
/// number is `host_serial`: five characters from the host's, then the
/// ID in two. Seven letters and digits, as `QSRLNBR` holds; no two
/// partitions of a host have one, and the same host gives the same ID
/// the same number.
pub fn serial_for(host_serial: &str, id: u32) -> String {
    // FNV-1a, 64 bits.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in host_serial.trim().bytes() {
        hash ^= u64::from(byte.to_ascii_uppercase());
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let radix = ALPHABET.len() as u64;
    let mut out = String::with_capacity(SERIAL_LEN);
    for _ in 0..SERIAL_LEN - 2 {
        // Below 34.
        #[allow(clippy::cast_possible_truncation)]
        out.push(ALPHABET[(hash % radix) as usize] as char);
        hash /= radix;
    }
    let id = id.min(MAX_ID) as usize;
    out.push(ALPHABET[id / ALPHABET.len()] as char);
    out.push(ALPHABET[id % ALPHABET.len()] as char);
    out
}

/// Whether `serial` is a system serial number.
pub fn valid_serial(serial: &str) -> bool {
    serial.len() == SERIAL_LEN && serial.bytes().all(|b| ALPHABET.contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_serial_number_is_seven_characters_of_the_host_and_the_id() {
        let first = serial_for("7XK2M41", 1);
        assert!(valid_serial(&first), "{first}");
        assert_eq!(first, serial_for(" 7xk2m41\n", 1));
        // The same host: the same five, the ID in the last two.
        let other = serial_for("7XK2M41", 35);
        assert_eq!(first[..5], other[..5]);
        assert_eq!(&first[5..], "01");
        assert_eq!(&other[5..], "11");
        assert_eq!(&serial_for("7XK2M41", MAX_ID)[5..], "VD");
        // Another host: another five.
        assert_ne!(first[..5], serial_for("7XK2M42", 1)[..5]);
        // No two IDs of a host share one.
        let mut all: Vec<String> = (1..=MAX_ID).map(|id| serial_for("H", id)).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), MAX_ID as usize);
        assert!(valid_serial(&serial_for("", 1)));
        assert!(!valid_serial("ABCDEF"));
        assert!(!valid_serial("ABCDEFI"));
        assert!(!valid_serial("abcdefg"));
    }

    #[test]
    fn a_definition_is_a_state_file() {
        let definition = Definition {
            id: 3,
            name: "PROD".into(),
            environment: Environment::Qs400,
            serial: serial_for("HOST", 3),
            keylock: Keylock::Manual,
            default_profile: "NORMAL".into(),
            auto_start: AutoStart {
                enabled: true,
                order: 10,
                delay_secs: 30,
            },
            created: 1_794_000_000,
        };
        let text = state::store::render(&definition).unwrap();
        assert!(text.contains("environment = \"QS/400\""), "{text}");
        assert!(text.contains("keylock = \"manual\""), "{text}");
        assert_eq!(
            state::store::parse::<Definition>(&text).unwrap(),
            definition
        );
        let back: Definition = ipc::decode(&ipc::encode(&definition)).unwrap();
        assert_eq!(back, definition);
        assert_eq!(user_of(3), 23003);
    }
}
