// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `seed.toml`: a `schema_version`, then one table per state file it
//! sets, each with the keys of that file. A table left out gives that
//! file's defaults. An installer's answers are further tables of a
//! later version.

use serde::Deserialize;
use state::{Document, Host, Network};

/// The file on the seed volume.
pub const FILE: &str = "seed.toml";
/// The version this code reads.
pub const VERSION: u32 = 1;
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
}

/// A checked seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seed {
    /// What `host.toml` becomes.
    pub host: Host,
    /// What `network.toml` becomes.
    pub network: Network,
}

/// A seed from the text of `seed.toml`, or why it is refused.
pub fn parse(text: &str) -> Result<Seed, String> {
    let file: File = toml::from_str(text).map_err(|e| format!("{FILE}: {e}"))?;
    if file.schema_version != VERSION {
        return Err(format!(
            "{FILE}: schema version {} is not {VERSION}, the one this release reads",
            file.schema_version
        ));
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::{Device, Mode};

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
            ("schema_version = 2\n", "schema version 2"),
            ("schema_version = 1\n[host]\nname = \"-\"\n", "[host]"),
            ("schema_version = 1\n[host]\nserial = \"X\"\n", "serial"),
            (
                "schema_version = 1\n[network]\nmode = \"static\"\n",
                "[network]",
            ),
            ("schema_version = 1\n[network]\nvlan = 0\n", "[network]"),
            ("schema_version = 1\n[install]\ndisk = \"sda\"\n", "install"),
            ("schema_version = 1\n[host]\nnmae = \"a\"\n", "nmae"),
            ("schema_version = 1\n[host\n", "seed.toml"),
        ] {
            let e = parse(text).unwrap_err();
            assert!(e.contains(place), "{text:?}: {e}");
        }
    }
}
