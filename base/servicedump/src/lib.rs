// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An appliance's service dump: service data in a ZIP archive any
//! workstation opens, with a manifest of what is in it. No secret goes
//! in: the identity's key and the users' password hashes are left out
//! by name, with the product's own secrets, and any file that holds a
//! private key or a password hash whatever its name.
//!
//! - [`zip`]: the archive, stored entries with their CRC-32.
//! - [`store`]: the dumps on the data file system, named by the
//!   product's prefix and their time, bounded by its kind.
//! - [`take`]: what every appliance gathers, the product's own entries
//!   beside it, written.

#![forbid(unsafe_code)]

pub mod store;
pub mod take;
pub mod zip;

pub use store::{Info, Kind, Store};
pub use take::{Asked, LEFT_OUT, Paths, Sources, take};

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    /// The product's own entries, and a secret file of its own.
    struct Own;

    impl Sources for Own {
        fn entries(&mut self) -> Vec<(String, Vec<u8>)> {
            vec![("own/report.txt".into(), b"all well".to_vec())]
        }
    }

    const KIND: Kind = Kind {
        prefix: "XMPDUMP-",
        title: "Service dump",
        max_dumps: 4,
        max_total: 1 << 30,
        left_out: &[("pins.toml", "the consoles' pins")],
    };

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("servicedump-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A machine's directories as the base services write them, every
    /// file that keeps a secret among them.
    fn machine(d: &Path) -> Paths {
        let state = d.join("state");
        fs::create_dir_all(state.join("things/1")).unwrap();
        identity::ensure(&state, "DEMO", "S1", 1_794_834_942).unwrap();
        fs::write(
            state.join(users::FILE),
            users::first("ADMIN", &users::hash("Passw0rd!x").unwrap(), 1).unwrap(),
        )
        .unwrap();
        fs::write(state.join("host.toml"), "name = \"DEMO\"\n").unwrap();
        fs::write(state.join("things/1/thing.toml"), "name = \"ONE\"\n").unwrap();
        fs::write(state.join("pins.toml"), "[[console]]\n").unwrap();
        let log = d.join("log");
        fs::create_dir_all(&log).unwrap();
        fs::write(log.join("refcodes.ndjson"), "{\"code\":\"C1001000\"}\n").unwrap();
        let problems = d.join("problems");
        fs::create_dir_all(&problems).unwrap();
        fs::write(problems.join("problems.json"), "{\"next\":1}").unwrap();
        Paths {
            log,
            state,
            problems,
            proc: d.join("proc"),
        }
    }

    #[test]
    fn a_dump_holds_what_its_manifest_says_and_no_secret() {
        let d = scratch("take");
        let paths = machine(&d);
        let store = Store::at(d.join("dumps"), &KIND);
        let asked = Asked {
            by: "Service Tools, user ADMIN".into(),
            release: "DEMO V1R0M0".into(),
            host: "DEMO S1".into(),
            status: "logd ready".into(),
        };
        let (info, _) = take(&store, &paths, &mut Own, &asked, 1_794_834_942).unwrap();
        assert_eq!(info.name, "XMPDUMP-20261116-131542.zip");
        let bytes = fs::read(d.join("dumps").join(&info.name)).unwrap();
        let entries = zip::entries(&bytes).unwrap();
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        for want in [
            "log/refcodes.ndjson",
            "problems/problems.json",
            "state/host.toml",
            "state/identity.crt",
            "state/things/1/thing.toml",
            "services/status.txt",
            "own/report.txt",
            "MANIFEST.txt",
        ] {
            assert!(names.contains(&want), "{want} not in {names:?}");
        }
        for (name, data) in &entries {
            assert!(!take::secret(name, data, KIND.left_out), "{name}");
        }
        for gone in [identity::KEY_FILE, users::FILE, "pins.toml"] {
            assert!(!names.iter().any(|n| n.ends_with(gone)), "{gone}");
        }
        let manifest = String::from_utf8_lossy(&entries.last().unwrap().1).into_owned();
        assert!(manifest.starts_with("Service dump XMPDUMP-"), "{manifest}");
        assert!(manifest.contains("state/identity.key: the host's private key"));
        assert!(manifest.contains("state/pins.toml: the consoles' pins"));
        assert_eq!(store.list()[0].by, "Service Tools, user ADMIN");
        fs::remove_dir_all(&d).unwrap();
    }

    /// Every file a base crate keeps a secret in is on the list by its
    /// name: a new one fails the build until it is added.
    #[test]
    fn every_file_a_base_crate_keeps_a_secret_in_is_left_out_by_name() {
        let named: Vec<&str> = LEFT_OUT.iter().map(|(n, _)| *n).collect();
        for secret in [identity::KEY_FILE, users::FILE] {
            assert!(named.contains(&secret), "{secret} is not on the list");
        }
    }
}
