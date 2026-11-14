// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The configuration a VMM is started with: a profile, resolved.

use std::collections::BTreeMap;

use model::{ConsoleMode, Definition, IplSource, IplType, Keylock, Profile};
use qsvmm::config::{Adapter, Config, Net, Optical, TapeDrive, Unit};
use resource::{Current, Vcpus};

/// What the units and devices are kept in, as `qsvmm` names them: a
/// file given to it (`/proc/self/fd/N`) or a host path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stores {
    /// A unit's backing store, by its slot and unit number.
    pub units: BTreeMap<(u32, u32), String>,
    /// An optical device's medium by its slot: the file and its name.
    pub optical: BTreeMap<u32, (String, String)>,
    /// A tape device's volume by its slot: the file, its name, its
    /// capacity in MiB and its write protection.
    pub tape: BTreeMap<u32, (String, String, u64, bool)>,
    /// How many files the pool service gave: the taps' follow them.
    pub files: usize,
    /// The Ethernet adapters, each with its tap or none.
    pub net: Vec<Net>,
}

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
    stores: &Stores,
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
                    backing: stores
                        .units
                        .get(&(adapter.slot, unit.lun))
                        .cloned()
                        .unwrap_or_else(|| unit.backing.clone())
                        .into(),
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
        optical: profile
            .optical
            .iter()
            .map(|o| {
                let medium = stores.optical.get(&o.slot);
                Optical {
                    slot: o.slot,
                    serial: unit_serial(definition, o.slot, 0),
                    media: medium.map(|(path, _)| path.into()),
                    name: medium.map(|(_, name)| name.clone()).unwrap_or_default(),
                }
            })
            .collect(),
        tape: profile
            .tape
            .iter()
            .map(|t| {
                let volume = stores.tape.get(&t.slot);
                TapeDrive {
                    slot: t.slot,
                    serial: unit_serial(definition, t.slot, 0),
                    volume: volume.map(|(path, ..)| path.into()),
                    name: volume.map(|(_, name, ..)| name.clone()).unwrap_or_default(),
                    capacity_mib: volume.map_or(0, |(_, _, cap, _)| *cap),
                    protected: volume.is_some_and(|(.., p)| *p),
                }
            })
            .collect(),
        net: stores.net.clone(),
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
            &Stores::default(),
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
            &Stores::default(),
        );
        assert_eq!((config.vcpus, config.pin.len()), (6, 0));
        assert_eq!(config.side, "b");
        assert!(qsvmm::config::check(config).is_ok());
    }
}
