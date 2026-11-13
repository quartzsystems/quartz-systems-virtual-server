// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A definition and two profiles that are valid: what the tests start
//! from, and what `docs/PROFILE.md` shows.

use crate::definition::{AutoStart, Definition, Environment, Keylock, serial_for};
use crate::profile::{Boot, Lun, Memory, ProcessorMode, Processors, Profile, ScsiAdapter, Sharing};
use crate::units::Units;

/// A partition's definition.
pub fn definition() -> Definition {
    Definition {
        id: 1,
        name: "PROD".into(),
        environment: Environment::Qs400,
        serial: serial_for("SAMPLE", 1),
        keylock: Keylock::Normal,
        default_profile: "NORMAL".into(),
        auto_start: AutoStart {
            enabled: true,
            order: 10,
            delay_secs: 0,
        },
        created: 1_794_000_000,
    }
}

fn storage(backing: &str) -> Vec<ScsiAdapter> {
    vec![ScsiAdapter {
        slot: 2,
        luns: vec![Lun {
            lun: 0,
            backing: backing.into(),
            serial: Some("PROD-LS".into()),
        }],
    }]
}

/// A profile with dedicated processors.
pub fn dedicated() -> Profile {
    Profile {
        name: "NORMAL".into(),
        processors: Processors {
            mode: ProcessorMode::Dedicated,
            minimum: 1,
            desired: 2,
            maximum: 4,
            minimum_units: None,
            desired_units: None,
            maximum_units: None,
            sharing: None,
            weight: None,
            pool: None,
        },
        memory: Memory {
            minimum_mib: 1024,
            desired_mib: 2048,
            maximum_mib: 4096,
        },
        boot: Boot::default(),
        scsi: storage("/dev/sdb"),
        ethernet: Vec::new(),
        optical: Vec::new(),
        tape: Vec::new(),
        physical_io: Vec::new(),
    }
}

/// A profile with shared, uncapped processors.
pub fn shared() -> Profile {
    Profile {
        name: "SHARED".into(),
        processors: Processors {
            mode: ProcessorMode::Shared,
            minimum: 1,
            desired: 2,
            maximum: 4,
            minimum_units: Some(Units(50)),
            desired_units: Some(Units(150)),
            maximum_units: Some(Units(400)),
            sharing: Some(Sharing::Uncapped),
            weight: Some(128),
            pool: None,
        },
        ..dedicated()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_a_state_file_and_travels() {
        for profile in [dedicated(), shared()] {
            let text = state::store::render(&profile).unwrap();
            assert_eq!(state::store::parse::<Profile>(&text).unwrap(), profile);
            let back: Profile = ipc::decode(&ipc::encode(&profile)).unwrap();
            assert_eq!(back, profile);
        }
        let text = state::store::render(&shared()).unwrap();
        for line in [
            "schema_version = 1",
            "mode = \"shared\"",
            "desired_units = 1.5",
            "sharing = \"uncapped\"",
            "source = \"recorded\"",
            "type = \"unattended\"",
            "console = \"serial\"",
            "[[scsi]]",
            "[[scsi.lun]]",
        ] {
            assert!(text.contains(line), "no `{line}` in:\n{text}");
        }
        // A key no profile has is refused, not passed over.
        let unknown = text.replace("desired_mib", "wanted_mib");
        assert!(state::store::parse::<Profile>(&unknown).is_err());
    }
}
