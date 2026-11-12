// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The seed: how a host gets its first configuration before there is
//! a console to enter it on. On an IPL whose state directory holds no
//! `host.toml`, PID 1 looks for a FAT volume with the appliance's seed
//! label, reads `seed.toml` from it, checks it against the state
//! schemas and writes the state files.
//!
//! - [`format`]: what `seed.toml` holds.
//! - [`volume`]: finding and reading the volume (Linux).

#![forbid(unsafe_code)]

pub mod format;
pub mod volume;

use state::{Host, StateError, Store};

pub use format::Seed;

/// What the seed step did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The host is configured already: nothing was looked for.
    NotNeeded,
    /// No seed volume.
    Absent,
    /// The state files were written from the seed.
    Applied,
    /// A seed was found and refused; nothing was written.
    Invalid(String),
}

/// Whether a seed applies: the store has no `host.toml`.
pub fn wanted(store: &Store) -> bool {
    !store.has::<Host>()
}

/// Write the state files of `seed`. `host.toml` is last: it is what
/// says the host has been seeded.
pub fn apply(seed: &Seed, store: &Store) -> Result<(), StateError> {
    store.save(&seed.network)?;
    store.save(&seed.host)
}

/// The seed step, given the text of `seed.toml` if a volume had one.
pub fn step(found: Option<Result<String, String>>, store: &Store) -> Outcome {
    match found {
        None => Outcome::Absent,
        Some(Err(reason)) => Outcome::Invalid(reason),
        Some(Ok(text)) => match format::parse(&text) {
            Err(reason) => Outcome::Invalid(reason),
            Ok(seed) => match apply(&seed, store) {
                Ok(()) => Outcome::Applied,
                Err(e) => Outcome::Invalid(e.to_string()),
            },
        },
    }
}

/// The seed step of an IPL: look for the volume labelled `label` when
/// `store` has no `host.toml`, mounting it at `mount_at` to read it.
#[cfg(target_os = "linux")]
pub fn run(label: &str, mount_at: &std::path::Path, store: &Store) -> Outcome {
    if !wanted(store) {
        return Outcome::NotNeeded;
    }
    let found = volume::find(label).map(|device| volume::read(&device, mount_at));
    step(found, store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::Network;

    fn scratch(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("seed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::at(dir)
    }

    const GOOD: &str = "schema_version = 1\n[host]\nname = \"vs-01\"\n\
                        [network]\nmode = \"static\"\naddress = \"192.0.2.10/24\"\nvlan = 7\n";

    #[test]
    fn a_good_seed_writes_both_files() {
        let store = scratch("good");
        assert!(wanted(&store));
        assert_eq!(step(Some(Ok(GOOD.into())), &store), Outcome::Applied);
        assert!(!wanted(&store));
        let host: Host = store.load().unwrap().unwrap();
        assert_eq!(host.name.as_deref(), Some("vs-01"));
        let net: Network = store.load().unwrap().unwrap();
        assert_eq!(net.vlan, Some(7));
        std::fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn a_bad_seed_writes_nothing() {
        let store = scratch("bad");
        let bad = GOOD.replace("vlan = 7", "vlan = 5000");
        let Outcome::Invalid(reason) = step(Some(Ok(bad)), &store) else {
            panic!("applied");
        };
        assert!(reason.contains("5000"), "{reason}");
        assert!(wanted(&store));
        assert!(!store.dir().exists());
        assert_eq!(
            step(Some(Err("unreadable".into())), &store),
            Outcome::Invalid("unreadable".into())
        );
        assert_eq!(step(None, &store), Outcome::Absent);
    }
}
