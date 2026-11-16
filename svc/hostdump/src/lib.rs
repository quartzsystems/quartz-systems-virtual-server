// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's own dump (`docs/DUMPS.md`): service data in a ZIP archive
//! any workstation opens, with a manifest of what is in it. No secret
//! goes in: the identity's key, the users' password hashes and the
//! consoles' pins are left out by name, and any file that holds a
//! private key or a password hash whatever its name.
//!
//! - [`zip`]: the archive, stored entries with their CRC-32.
//! - [`store`]: the dumps on `/data`, named by their time, bounded.
//! - [`take`]: what a dump holds, gathered and written.
//! - `live` (Linux): what the services say, from their sockets.

#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
pub mod live;
pub mod store;
pub mod take;
pub mod zip;

pub use store::{Info, Store};
pub use take::{Asked, LEFT_OUT, Paths, Sources, take};

/// What QSI calls the kind of dump this is.
pub const KIND_NAME: &str = "service data";

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    struct Said;

    impl Sources for Said {
        fn partitions(&mut self) -> String {
            "PROD running".into()
        }
        fn pools(&mut self) -> String {
            "POOL1 mirrored".into()
        }
        fn switches(&mut self) -> String {
            "LAN 1500".into()
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("hostdump-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A host's directories as the services write them, every file that
    /// keeps a secret among them.
    fn host(d: &Path) -> Paths {
        let state = d.join("state");
        fs::create_dir_all(state.join("partitions/1")).unwrap();
        identity::ensure(&state, "VSHOST", "S1", 1_794_834_942).unwrap();
        fs::write(
            state.join(users::FILE),
            users::first("QSECOFR", &users::hash("Passw0rd!x").unwrap(), 1).unwrap(),
        )
        .unwrap();
        fs::write(state.join("host.toml"), "name = \"VSHOST\"\n").unwrap();
        fs::write(
            state.join("partitions/1/definition.toml"),
            "name = \"PROD\"\n",
        )
        .unwrap();
        fs::write(
            state.join("qsi-consoles.toml"),
            "[[console]]\nname = \"MC\"\n",
        )
        .unwrap();
        let log = d.join("log");
        fs::create_dir_all(&log).unwrap();
        fs::write(log.join("refcodes.ndjson"), "{\"code\":\"C2001000\"}\n").unwrap();
        fs::write(log.join("system.ndjson"), "{\"message\":\"hello\"}\n").unwrap();
        let problems = d.join("problems");
        fs::create_dir_all(&problems).unwrap();
        fs::write(problems.join("problems.json"), "{\"next\":1}").unwrap();
        Paths {
            log,
            state,
            problems,
            cgroup: d.join("cgroup"),
            proc: d.join("proc"),
        }
    }

    #[test]
    fn a_dump_holds_what_its_manifest_says_and_no_secret() {
        let d = scratch("take");
        let paths = host(&d);
        let store = Store::at(d.join("dumps"));
        let asked = Asked {
            by: "Service Tools, user QSECOFR".into(),
            release: "QS/VS V1R0M0".into(),
            host: "VSHOST S1".into(),
            status: "logd ready".into(),
        };
        let (info, _) = take(&store, &paths, &mut Said, &asked, 1_794_834_942).unwrap();
        assert_eq!(info.name, "HOSTDUMP-20261116-131542.zip");
        let bytes = fs::read(d.join("dumps").join(&info.name)).unwrap();
        let entries = zip::entries(&bytes).unwrap();
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        for want in [
            "log/refcodes.ndjson",
            "log/system.ndjson",
            "problems/problems.json",
            "state/host.toml",
            "state/identity.crt",
            "state/partitions/1/definition.toml",
            "services/status.txt",
            "services/partitions.txt",
            "services/pools.txt",
            "services/switches.txt",
            "host/cgroups.txt",
            "MANIFEST.txt",
        ] {
            assert!(names.contains(&want), "{want} not in {names:?}");
        }
        // No secret, by name or by what it holds.
        for (name, data) in &entries {
            assert!(!take::secret(name, data), "{name} holds a secret");
            let text = String::from_utf8_lossy(data);
            assert!(
                !text.contains("PRIVATE KEY") && !text.contains("$argon2"),
                "{name}"
            );
        }
        for gone in [identity::KEY_FILE, users::FILE, "qsi-consoles.toml"] {
            assert!(!names.iter().any(|n| n.ends_with(gone)), "{gone}");
        }
        // The manifest names each entry and what was left out, and why.
        let manifest = String::from_utf8_lossy(&entries.last().unwrap().1).into_owned();
        assert!(manifest.contains("state/host.toml"));
        assert!(manifest.contains("state/identity.key: the host's private key"));
        assert!(manifest.contains("By Service Tools, user QSECOFR"));
        assert_eq!(store.list()[0].by, "Service Tools, user QSECOFR");
        fs::remove_dir_all(&d).unwrap();
    }

    /// Every file a crate keeps a secret in is on the list by its name:
    /// a new one fails the build until it is added.
    #[test]
    fn every_file_that_keeps_a_secret_is_left_out_by_name() {
        let named: Vec<&str> = LEFT_OUT.iter().map(|(n, _)| *n).collect();
        for secret in [identity::KEY_FILE, users::FILE, qsiserver_consoles()] {
            assert!(named.contains(&secret), "{secret} is not on the list");
        }
    }

    /// The QSI server's pins file, by the name the server gives it.
    fn qsiserver_consoles() -> &'static str {
        "qsi-consoles.toml"
    }
}
