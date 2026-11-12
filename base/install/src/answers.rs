// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An unattended installation: the `[install]` table of a seed file,
//! with its `[host]` and `[network]` tables, held against the machine
//! it is to be carried out on.

use panels::forms::TimeChoice;
use seed::Seed;
use seed::format::Install;

use crate::{Answers, DiskChoice, Target};

/// The disk `wanted` names, by serial number or location code.
fn disk<'a>(disks: &'a [DiskChoice], wanted: &str) -> Result<&'a DiskChoice, String> {
    let wanted = wanted.trim();
    let found: Vec<&DiskChoice> = disks
        .iter()
        .filter(|d| d.serial == wanted || d.location == wanted)
        .collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("there is no disk {wanted}")),
        _ => Err(format!("{} disks answer to {wanted}", found.len())),
    }
}

/// The answers `seed` gives for `target`'s machine, or why they cannot
/// be used. A seed with no `[install]` table gives none.
pub fn from_seed(seed: &Seed, target: &mut dyn Target) -> Result<Option<Answers>, String> {
    let Some(install) = &seed.install else {
        return Ok(None);
    };
    resolve(seed, install, target).map(Some)
}

fn resolve(seed: &Seed, install: &Install, target: &mut dyn Target) -> Result<Answers, String> {
    let all = target.disks();
    let least = target.min_disk_bytes();
    let mut disks = Vec::new();
    for wanted in &install.disks {
        let found = disk(&all, wanted)?;
        if found.bytes < least {
            return Err(format!(
                "disk {wanted} is {} bytes; a load source needs {least}",
                found.bytes
            ));
        }
        if disks.contains(found) {
            return Err(format!("disk {wanted} is named twice"));
        }
        disks.push(found.clone());
    }
    let user_id = users::check_id(&install.user_id).map_err(|e| format!("user_id: {e}"))?;
    if !users::valid_hash(&install.password_hash) {
        return Err("password_hash is not an Argon2id hash".into());
    }
    Ok(Answers {
        disks,
        host: seed
            .host
            .name
            .clone()
            .unwrap_or_else(|| target.default_host()),
        network: seed.network.clone(),
        time: TimeChoice {
            zone: seed.host.time_zone.clone(),
            servers: seed.host.ntp_servers.clone(),
            clock: None,
        },
        user_id,
        password_hash: install.password_hash.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::tests::Machine;

    const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNo";

    fn seed(disks: &str, more: &str) -> Seed {
        seed::format::parse(&format!(
            "schema_version = 2\n[host]\nname = \"vs-01\"\ntime_zone = \"UTC+01:00\"\n\
             [install]\ndisks = {disks}\nuser_id = \"admin\"\npassword_hash = \"{HASH}\"\n{more}"
        ))
        .unwrap()
    }

    #[test]
    fn disks_are_found_by_serial_or_by_location() {
        let mut machine = Machine::default();
        let answers = from_seed(
            &seed("[\"SER-A\", \"P0000-B00-D06-F0-U0\"]", ""),
            &mut machine,
        )
        .unwrap()
        .unwrap();
        let names: Vec<&str> = answers.disks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["vda", "vdb"]);
        assert!(answers.mirrored());
        assert_eq!(
            (answers.host.as_str(), answers.user_id.as_str()),
            ("vs-01", "ADMIN")
        );
        assert_eq!(answers.time.zone, "UTC+01:00");
        assert_eq!(answers.password_hash, HASH);
        // No [install]: the installer asks.
        let plain = seed::format::parse("schema_version = 2\n").unwrap();
        assert_eq!(from_seed(&plain, &mut machine), Ok(None));
    }

    #[test]
    fn answers_that_do_not_fit_the_machine_are_refused() {
        let mut machine = Machine::default();
        for (disks, why) in [
            ("[\"SER-X\"]", "there is no disk SER-X"),
            ("[\"SER-SMALL\"]", "a load source needs"),
            ("[\"SER-A\", \"P0000-B00-D05-F0-U0\"]", "named twice"),
        ] {
            let e = from_seed(&seed(disks, ""), &mut machine).unwrap_err();
            assert!(e.contains(why), "{disks}: {e}");
        }
        let mut bad = seed("[\"SER-A\"]", "");
        bad.install.as_mut().unwrap().user_id = "not an id".into();
        assert!(
            from_seed(&bad, &mut machine)
                .unwrap_err()
                .starts_with("user_id")
        );
        let mut bad = seed("[\"SER-A\"]", "");
        bad.install.as_mut().unwrap().password_hash = "$argon2id$nothing".into();
        assert!(
            from_seed(&bad, &mut machine)
                .unwrap_err()
                .contains("Argon2id")
        );
    }
}
