// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `seed.toml`: a `schema_version`, then one table per state file it
//! sets, each with the keys of that file. A table left out gives that
//! file's defaults. Version 2 adds `[install]`, an installer's answers:
//! with it the file is the answers file of an unattended installation.
//! A running host reads the other tables and leaves that one alone.

use serde::Deserialize;
use state::{Document, Host, Network};

/// The file on the seed volume.
pub const FILE: &str = "seed.toml";
/// The newest version this code reads. Version 1 has no `[install]`.
pub const VERSION: u32 = 2;
/// The longest seed file read.
pub const MAX_BYTES: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema_version: u32,
    #[serde(default)]
    host: Host,
    #[serde(default)]
    network: Network,
    install: Option<Install>,
}

/// How the load source is protected.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protection {
    /// One disk.
    None,
    /// Two disks, mirrored.
    Mirrored,
}

/// `[install]`: what an installer is told instead of being asked.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Install {
    /// The load-source disks, each by serial number or location code:
    /// one, or two for mirrored protection.
    pub disks: Vec<String>,
    /// The protection; left out, it follows from the number of disks.
    pub protection: Option<Protection>,
    /// The service tools user ID.
    pub user_id: String,
    /// The hash of that user's password, an Argon2id PHC string.
    pub password_hash: String,
}

impl Install {
    /// The protection asked for, or what the disks imply.
    pub fn protection(&self) -> Protection {
        self.protection.unwrap_or(if self.disks.len() == 2 {
            Protection::Mirrored
        } else {
            Protection::None
        })
    }

    fn validate(&self) -> Result<(), String> {
        let named = self.disks.iter().all(|d| !d.trim().is_empty());
        match (self.disks.as_slice(), self.protection()) {
            (_, _) if !named => Err("a disk has no name".into()),
            ([_], Protection::None) => Ok(()),
            ([a, b], Protection::Mirrored) if a != b => Ok(()),
            ([_, _], Protection::Mirrored) => Err("the two disks are the same".into()),
            (disks, Protection::None) => Err(format!(
                "{} disks without protection; it takes one",
                disks.len()
            )),
            (disks, Protection::Mirrored) => Err(format!(
                "{} disks for mirrored protection; it takes two",
                disks.len()
            )),
        }?;
        if self.user_id.trim().is_empty() {
            return Err("user_id is empty".into());
        }
        if !self.password_hash.starts_with("$argon2id$") {
            return Err("password_hash is not an Argon2id hash".into());
        }
        Ok(())
    }
}

/// A checked seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    /// What `host.toml` becomes.
    pub host: Host,
    /// What `network.toml` becomes.
    pub network: Network,
    /// An installer's answers, when the file has them.
    pub install: Option<Install>,
}

/// A seed from the text of `seed.toml`, or why it is refused.
pub fn parse(text: &str) -> Result<Seed, String> {
    let file: File = toml::from_str(text).map_err(|e| format!("{FILE}: {e}"))?;
    if file.schema_version == 0 || file.schema_version > VERSION {
        return Err(format!(
            "{FILE}: schema version {} is not one this release reads (1 to {VERSION})",
            file.schema_version
        ));
    }
    if let Some(install) = &file.install {
        if file.schema_version < 2 {
            return Err(format!("{FILE}: [install] needs schema_version = 2"));
        }
        install
            .validate()
            .map_err(|e| format!("{FILE}: [install]: {e}"))?;
    }
    file.host
        .validate()
        .map_err(|e| format!("{FILE}: [host]: {e}"))?;
    file.network
        .validate()
        .map_err(|e| format!("{FILE}: [network]: {e}"))?;
    Ok(Seed {
        host: file.host,
        network: file.network,
        install: file.install,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::{Device, Mode};

    fn install(disks: &str, rest: &str) -> Result<Seed, String> {
        parse(&format!(
            "schema_version = 2\n[install]\ndisks = {disks}\nuser_id = \"ADMIN\"\n\
             password_hash = \"$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA\"\n{rest}"
        ))
    }

    #[test]
    fn the_answers_of_an_installation_parse() {
        let seed = install("[\"QSVSHD0\", \"P0000-B00-D06-F0-U0\"]", "").unwrap();
        let answers = seed.install.unwrap();
        assert_eq!(answers.disks.len(), 2);
        assert_eq!(answers.protection(), Protection::Mirrored);
        assert_eq!(answers.user_id, "ADMIN");
        let one = install("[\"QSVSHD0\"]", "protection = \"none\"\n").unwrap();
        assert_eq!(one.install.unwrap().protection(), Protection::None);
        // A version 2 file need not have them.
        assert_eq!(parse("schema_version = 2\n").unwrap().install, None);
    }

    #[test]
    fn answers_that_do_not_fit_are_refused() {
        for (disks, rest, why) in [
            ("[]", "", "0 disks"),
            ("[\"A\", \"B\", \"C\"]", "", "3 disks"),
            ("[\"A\", \"A\"]", "", "the same"),
            ("[\"A\"]", "protection = \"mirrored\"\n", "takes two"),
            ("[\"A\", \"B\"]", "protection = \"none\"\n", "takes one"),
            ("[\"A\", \" \"]", "", "no name"),
            ("[\"A\"]", "protection = \"parity\"\n", "parity"),
            ("[\"A\"]", "password = \"clear\"\n", "password"),
        ] {
            let e = install(disks, rest).unwrap_err();
            assert!(e.contains(why), "{disks} {rest:?}: {e}");
        }
        let plain = "schema_version = 2\n[install]\ndisks = [\"A\"]\nuser_id = \"ADMIN\"\n\
                     password_hash = \"secret\"\n";
        assert!(parse(plain).unwrap_err().contains("Argon2id"));
    }

    #[test]
    fn a_whole_seed_parses() {
        let seed = parse(
            "schema_version = 1\n\n[host]\nname = \"vs-01\"\ntime_zone = \"UTC+01:00\"\n\
             ntp_servers = [\"10.0.2.2:12300\"]\n\n[network]\ndevice = \"52:54:00:12:34:56\"\n\
             mode = \"static\"\naddress = \"192.0.2.10/24\"\ngateway = \"192.0.2.1\"\nvlan = 100\n\
             dns = [\"192.0.2.53\"]\n",
        )
        .unwrap();
        assert_eq!(seed.host.name.as_deref(), Some("vs-01"));
        assert_eq!(seed.host.ntp_servers, ["10.0.2.2:12300"]);
        assert_eq!(seed.network.mode, Mode::Static);
        assert_eq!(seed.network.vlan, Some(100));
    }

    #[test]
    fn tables_left_out_are_the_defaults() {
        let seed = parse("schema_version = 1\n").unwrap();
        assert_eq!(seed.host, Host::default());
        assert_eq!(seed.network.device, Device::Auto);
        let seed = parse("schema_version = 1\n[host]\nname = \"a\"\n").unwrap();
        assert_eq!(seed.network, Network::default());
    }

    #[test]
    fn what_is_wrong_is_refused_with_its_place() {
        for (text, place) in [
            ("", "schema_version"),
            ("schema_version = 3\n", "schema version 3"),
            ("schema_version = 0\n", "schema version 0"),
            ("schema_version = 1\n[host]\nname = \"-\"\n", "[host]"),
            ("schema_version = 1\n[host]\nserial = \"X\"\n", "serial"),
            (
                "schema_version = 1\n[network]\nmode = \"static\"\n",
                "[network]",
            ),
            ("schema_version = 1\n[network]\nvlan = 0\n", "[network]"),
            ("schema_version = 1\n[install]\ndisk = \"sda\"\n", "install"),
            (
                "schema_version = 1\n[install]\ndisks = [\"A\"]\nuser_id = \"ADMIN\"\n\
                 password_hash = \"$argon2id$x\"\n",
                "needs schema_version = 2",
            ),
            ("schema_version = 1\n[host]\nnmae = \"a\"\n", "nmae"),
            ("schema_version = 1\n[host\n", "seed.toml"),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains(place), "{text:?}: {e}");
        }
    }
}
