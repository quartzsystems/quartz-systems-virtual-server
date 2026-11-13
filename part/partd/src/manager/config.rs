// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The configuration a VMM is started with: a profile, resolved.

use model::{ConsoleMode, Definition, IplSource, IplType, Keylock, Profile};
use qsvmm::config::{Adapter, Config, Unit};
use resource::{Current, Vcpus};

/// The serial number of a unit whose profile gives none: the
/// partition's, the slot and the unit.
pub fn unit_serial(definition: &Definition, slot: u32, lun: u32) -> String {
    format!("{}-{slot}-{lun}", definition.serial)
}

/// What `qsvmm` is told for one activation of `definition` with
/// `profile`, holding `current` as `vcpus`, its IPL from `source` with
/// the keylock at `keylock`.
pub fn build(
    definition: &Definition,
    profile: &Profile,
    current: &Current,
    vcpus: &Vcpus,
    source: IplSource,
    keylock: Keylock,
) -> Config {
    let adapters = profile
        .scsi
        .iter()
        .map(|adapter| Adapter {
            slot: adapter.slot,
            units: adapter
                .luns
                .iter()
                .map(|unit| Unit {
                    lun: unit.lun,
                    backing: unit.backing.clone().into(),
                    serial: unit
                        .serial
                        .clone()
                        .unwrap_or_else(|| unit_serial(definition, adapter.slot, unit.lun)),
                })
                .collect(),
        })
        .collect();
    Config {
        id: definition.id,
        name: definition.name.clone(),
        serial: definition.serial.clone(),
        vcpus: vcpus.count,
        threads_per_core: vcpus.threads_per_core,
        pin: vcpus.pin.clone(),
        memory_mib: current.memory_mib,
        huge_pages: true,
        side: source.word().to_string(),
        keylock: Some(keylock.word().to_string()),
        ipl_type: Some(
            match profile.boot.ipl_type {
                IplType::Unattended => "unattended",
                IplType::Attended => "attended",
            }
            .to_string(),
        ),
        console: Some(
            match profile.boot.console {
                ConsoleMode::Serial => "serial",
                ConsoleMode::Local => "local",
            }
            .to_string(),
        ),
        adapters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::sample;
    use resource::{Host, Processors, Settings, Topology};

    #[test]
    fn a_resolved_profile_is_a_configuration_qsvmm_takes() {
        let host = Host {
            topology: Topology::uniform(8, 2),
            installed_mib: 16_384,
            configurable_mib: 14_336,
            settings: Settings::default(),
        };
        let definition = sample::definition();
        let mut profile = sample::dedicated();
        profile.scsi[0].luns.push(model::Lun {
            lun: 1,
            backing: "/dev/sdc".into(),
            serial: None,
        });
        let current = host.resolve(&[], "PROD", &profile).unwrap();
        let vcpus = host.vcpus(&current.processors);
        let config = build(
            &definition,
            &profile,
            &current,
            &vcpus,
            IplSource::Recorded,
            Keylock::Manual,
        );
        assert_eq!(config.vcpus, 4);
        assert_eq!(config.threads_per_core, 2);
        assert_eq!(config.pin, [6, 14, 7, 15]);
        assert_eq!(config.memory_mib, 2048);
        assert!(config.huge_pages);
        assert_eq!(config.adapters[0].units[0].serial, "PROD-LS");
        assert_eq!(
            config.adapters[0].units[1].serial,
            format!("{}-2-1", definition.serial)
        );
        // And `qsvmm` reads what was written.
        let text = qsvmm::config::render(&config);
        let partition = qsvmm::config::parse(&text).unwrap();
        assert_eq!(partition.config, config);
        assert_eq!(partition.user(), model::definition::user_of(definition.id));

        // Shared: a virtual processor is a core's worth of virtual CPUs.
        let shared = Current {
            processors: Processors::Shared {
                pool: "DEFAULT".into(),
                units: model::Units(50),
                virtuals: 3,
                capped: true,
                weight: 0,
            },
            memory_mib: 1024,
        };
        let vcpus = host.vcpus(&shared.processors);
        let config = build(
            &definition,
            &sample::shared(),
            &shared,
            &vcpus,
            IplSource::B,
            Keylock::Normal,
        );
        assert_eq!((config.vcpus, config.pin.len()), (6, 0));
        assert_eq!(config.side, "b");
        assert!(qsvmm::config::check(config).is_ok());
    }
}
