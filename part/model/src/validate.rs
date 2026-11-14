// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Validation: every error of a definition or a profile at once, each
//! a message. Nothing here looks at the host: whether what a profile
//! asks for is there is an activation's question.

use qvsmsg::Issued;

use crate::definition::{Definition, MAX_DELAY_SECS, MAX_ID, valid_serial};
use crate::profile::{
    Lun, MAX_WEIGHT, MediaDevice, ProcessorMode, Processors, Profile, ScsiAdapter,
};
use crate::units::Units;

/// A name's most characters.
pub const NAME_MAX: usize = 10;
/// The most processors, or virtual processors, a profile may name.
pub const MAX_PROCESSORS: u32 = 240;
/// The least memory, in MiB.
pub const MIN_MEMORY_MIB: u64 = 128;
/// The most memory, in MiB: a tebibyte.
pub const MAX_MEMORY_MIB: u64 = 1 << 20;
/// The first virtual slot a profile may use: 0 and 1 are the
/// partition's console and service channel.
pub const FIRST_SLOT: u32 = 2;
/// The last virtual slot.
pub const LAST_SLOT: u32 = 255;
/// The most virtual SCSI adapters.
pub const MAX_SCSI_ADAPTERS: usize = 8;
/// The most units of one adapter.
pub const MAX_LUNS: usize = 64;
/// A unit's serial number's most characters.
pub const UNIT_SERIAL_MAX: usize = 20;

/// The most virtual Ethernet adapters of a profile.
pub const MAX_ETHERNET: usize = 8;
/// The most tagged VLANs of one adapter.
pub const MAX_TAGGED: usize = 32;

/// What validation needs of the host's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The memory block size, in MiB: memory is in multiples of it.
    pub memory_block_mib: u64,
}

/// The naming rule, of partitions, profiles and pools alike: 1 to 10
/// characters of `A` to `Z`, `0` to `9` and `_`, a letter first.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && name.as_bytes()[0].is_ascii_uppercase()
}

/// Whether `serial` is a disk unit's serial number: 1 to 20 letters,
/// digits, `-` and `_`.
pub fn valid_unit_serial(serial: &str) -> bool {
    (1..=UNIT_SERIAL_MAX).contains(&serial.len())
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `mac` is `xx:xx:xx:xx:xx:xx`, and no group address.
pub fn valid_mac(mac: &str) -> bool {
    let octets: Vec<&str> = mac.split(':').collect();
    octets.len() == 6
        && octets
            .iter()
            .all(|o| o.len() == 2 && o.bytes().all(|b| b.is_ascii_hexdigit()))
        && u8::from_str_radix(octets[0], 16).is_ok_and(|first| first & 1 == 0)
}

/// Whether `path` is an absolute path with nothing odd in it.
fn valid_path(path: &str) -> bool {
    path.starts_with('/') && path.len() > 1 && !path.contains('\0') && !path.contains("/../")
}

/// What a unit is kept in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backing<'a> {
    /// A virtual disk of a storage pool.
    Vdisk {
        /// The pool.
        pool: &'a str,
        /// The disk.
        name: &'a str,
    },
    /// A file or a block device of the host: a test image's.
    Path(&'a str),
}

/// Whether `name` is a virtual disk's or a medium's name: 1 to 32
/// letters, digits, `-` and `_`, a letter first.
pub fn valid_store_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && name.as_bytes()[0].is_ascii_alphabetic()
}

/// A unit's backing store as written: `POOL/NAME`, or an absolute path.
pub fn backing(written: &str) -> Option<Backing<'_>> {
    if written.starts_with('/') {
        return valid_path(written).then_some(Backing::Path(written));
    }
    let (pool, name) = written.split_once('/')?;
    (valid_name(pool) && valid_store_name(name)).then_some(Backing::Vdisk { pool, name })
}

/// Every error of a definition.
pub fn definition(definition: &Definition) -> Vec<Issued> {
    let mut errors = Vec::new();
    if definition.id == 0 || definition.id > MAX_ID {
        errors.push(qvsmsg::ID_INVALID.with(&[&definition.id.to_string(), &MAX_ID.to_string()]));
    }
    if !valid_name(&definition.name) {
        errors.push(qvsmsg::NAME_INVALID.with(&[&definition.name]));
    }
    if !valid_serial(&definition.serial) {
        errors.push(qvsmsg::SERIAL_INVALID.with(&[&definition.serial]));
    }
    if !definition.default_profile.is_empty() && !valid_name(&definition.default_profile) {
        errors.push(qvsmsg::PROFILE_NAME_INVALID.with(&[&definition.default_profile]));
    }
    if definition.auto_start.delay_secs > MAX_DELAY_SECS {
        errors.push(qvsmsg::DELAY_INVALID.with(&[
            &definition.auto_start.delay_secs.to_string(),
            &MAX_DELAY_SECS.to_string(),
        ]));
    }
    errors
}

fn processors(p: &Processors, errors: &mut Vec<Issued>) {
    let what = match p.mode {
        ProcessorMode::Dedicated => "processors",
        ProcessorMode::Shared => "virtual processors",
    };
    let (min, des, max) = (p.minimum, p.desired, p.maximum);
    if min == 0 || max > MAX_PROCESSORS {
        errors.push(qvsmsg::PROCESSORS_RANGE.with(&[
            what,
            &min.to_string(),
            &max.to_string(),
            &MAX_PROCESSORS.to_string(),
        ]));
    }
    if min > des || des > max {
        errors.push(qvsmsg::PROCESSORS_ORDER.with(&[
            what,
            &min.to_string(),
            &des.to_string(),
            &max.to_string(),
        ]));
    }
    match p.mode {
        ProcessorMode::Dedicated => {
            let shared_only = [
                ("minimum_units", p.minimum_units.is_some()),
                ("desired_units", p.desired_units.is_some()),
                ("maximum_units", p.maximum_units.is_some()),
                ("sharing", p.sharing.is_some()),
                ("weight", p.weight.is_some()),
                ("pool", p.pool.is_some()),
            ];
            for (field, _) in shared_only.iter().filter(|(_, there)| *there) {
                errors.push(qvsmsg::SHARED_ONLY.with(&[field]));
            }
        }
        ProcessorMode::Shared => {
            shared(p, errors);
        }
    }
}

fn shared(p: &Processors, errors: &mut Vec<Issued>) {
    let (Some(min), Some(des), Some(max)) = (p.minimum_units, p.desired_units, p.maximum_units)
    else {
        errors.push(qvsmsg::UNITS_MISSING.issue());
        return;
    };
    if min > des || des > max {
        errors.push(qvsmsg::UNITS_ORDER.with(&[
            &min.to_string(),
            &des.to_string(),
            &max.to_string(),
        ]));
    }
    // Each amount of units against the virtual processors of its own
    // level: at least 0.05 and at most 1.00 for each.
    for (level, units, virtuals) in [
        ("minimum", min, p.minimum),
        ("desired", des, p.desired),
        ("maximum", max, p.maximum),
    ] {
        let least = Units(virtuals.saturating_mul(Units::LEAST_PER_VIRTUAL.0));
        let most = Units::cores(virtuals);
        if units < least {
            errors.push(qvsmsg::UNITS_TOO_FEW.with(&[
                level,
                &units.to_string(),
                &virtuals.to_string(),
                &least.to_string(),
            ]));
        }
        if units > most {
            errors.push(qvsmsg::UNITS_TOO_MANY.with(&[
                level,
                &units.to_string(),
                &virtuals.to_string(),
                &most.to_string(),
            ]));
        }
    }
    if let Some(weight) = p.weight.filter(|w| *w > MAX_WEIGHT) {
        errors.push(qvsmsg::WEIGHT_INVALID.with(&[&weight.to_string(), &MAX_WEIGHT.to_string()]));
    }
    if let Some(pool) = p.pool.as_deref().filter(|pool| !valid_name(pool)) {
        errors.push(qvsmsg::POOL_NAME_INVALID.with(&[pool]));
    }
}

fn memory(profile: &Profile, limits: Limits, errors: &mut Vec<Issued>) {
    let m = &profile.memory;
    let block = limits.memory_block_mib.max(1);
    if m.minimum_mib > m.desired_mib || m.desired_mib > m.maximum_mib {
        errors.push(qvsmsg::MEMORY_ORDER.with(&[
            &m.minimum_mib.to_string(),
            &m.desired_mib.to_string(),
            &m.maximum_mib.to_string(),
        ]));
    }
    for (level, mib) in [
        ("minimum", m.minimum_mib),
        ("desired", m.desired_mib),
        ("maximum", m.maximum_mib),
    ] {
        if mib % block != 0 {
            errors.push(qvsmsg::MEMORY_BLOCK.with(&[level, &mib.to_string(), &block.to_string()]));
        }
        if !(MIN_MEMORY_MIB..=MAX_MEMORY_MIB).contains(&mib) {
            errors.push(qvsmsg::MEMORY_RANGE.with(&[
                level,
                &mib.to_string(),
                &MIN_MEMORY_MIB.to_string(),
                &MAX_MEMORY_MIB.to_string(),
            ]));
        }
    }
}

/// The slots of every kind of adapter: each is in range and used once.
fn slots(profile: &Profile, errors: &mut Vec<Issued>) {
    let media = |kind: &'static str, devices: &[MediaDevice]| -> Vec<(&'static str, u32)> {
        devices.iter().map(|d| (kind, d.slot)).collect()
    };
    let mut used: Vec<(&str, u32)> = profile.scsi.iter().map(|a| ("scsi", a.slot)).collect();
    used.extend(profile.ethernet.iter().map(|a| ("ethernet", a.slot)));
    used.extend(media("optical", &profile.optical));
    used.extend(media("tape", &profile.tape));
    for (i, (kind, slot)) in used.iter().enumerate() {
        if !(FIRST_SLOT..=LAST_SLOT).contains(slot) {
            errors.push(qvsmsg::SLOT_INVALID.with(&[
                kind,
                &slot.to_string(),
                &FIRST_SLOT.to_string(),
                &LAST_SLOT.to_string(),
            ]));
        }
        if let Some((first, _)) = used[..i].iter().find(|(_, s)| s == slot) {
            errors.push(qvsmsg::SLOT_TWICE.with(&[&slot.to_string(), first, kind]));
        }
    }
}

fn storage(profile: &Profile, errors: &mut Vec<Issued>) {
    if profile.scsi.len() > MAX_SCSI_ADAPTERS {
        errors.push(qvsmsg::TOO_MANY_ADAPTERS.with(&[
            &profile.scsi.len().to_string(),
            &MAX_SCSI_ADAPTERS.to_string(),
        ]));
    }
    if profile.scsi.iter().all(|a| a.luns.is_empty()) {
        errors.push(qvsmsg::NO_LOAD_SOURCE.issue());
    }
    let mut seen: Vec<(&ScsiAdapter, &Lun)> = Vec::new();
    for adapter in &profile.scsi {
        let slot = adapter.slot.to_string();
        // The guest finds a unit by its place: no number is left out.
        let count = adapter.luns.len();
        let in_order = adapter.luns.iter().all(|u| (u.lun as usize) < count);
        if count <= MAX_LUNS && count > 0 && !in_order {
            errors.push(qvsmsg::LUN_GAP.with(&[
                &slot,
                &count.to_string(),
                &(count - 1).to_string(),
            ]));
        }
        if adapter.luns.len() > MAX_LUNS {
            errors.push(qvsmsg::TOO_MANY_UNITS.with(&[
                &slot,
                &adapter.luns.len().to_string(),
                &MAX_LUNS.to_string(),
            ]));
        }
        for unit in &adapter.luns {
            let lun = unit.lun.to_string();
            if unit.lun as usize >= MAX_LUNS {
                errors.push(qvsmsg::LUN_INVALID.with(&[&slot, &lun, &(MAX_LUNS - 1).to_string()]));
            }
            if seen
                .iter()
                .any(|(a, u)| a.slot == adapter.slot && u.lun == unit.lun)
            {
                errors.push(qvsmsg::LUN_TWICE.with(&[&slot, &lun]));
            }
            if backing(&unit.backing).is_none() {
                errors.push(qvsmsg::BACKING_INVALID.with(&[&slot, &lun, &unit.backing]));
            } else if seen.iter().any(|(_, u)| u.backing == unit.backing) {
                errors.push(qvsmsg::BACKING_TWICE.with(&[&slot, &lun, &unit.backing]));
            }
            if let Some(serial) = &unit.serial {
                if !valid_unit_serial(serial) {
                    errors.push(qvsmsg::UNIT_SERIAL_INVALID.with(&[&slot, &lun, serial]));
                } else if seen.iter().any(|(_, u)| u.serial.as_ref() == Some(serial)) {
                    errors.push(qvsmsg::UNIT_SERIAL_TWICE.with(&[&slot, &lun, serial]));
                }
            }
            seen.push((adapter, unit));
        }
    }
}

/// The virtual Ethernet adapters: what needs no look at the host's
/// switches. Those are the partition manager's to check.
fn ethernet(profile: &Profile, errors: &mut Vec<Issued>) {
    if profile.ethernet.len() > MAX_ETHERNET {
        errors.push(qvsmsg::ETHERNET_TOO_MANY.with(&[
            &profile.ethernet.len().to_string(),
            &MAX_ETHERNET.to_string(),
        ]));
    }
    let mut macs: Vec<String> = Vec::new();
    for adapter in &profile.ethernet {
        let slot = adapter.slot.to_string();
        match adapter.switch.as_deref() {
            None => errors.push(qvsmsg::ADAPTER_SWITCH_NEEDED.with(&[&slot])),
            Some(s) if !valid_name(s) => {
                errors.push(qvsmsg::ADAPTER_SWITCH_INVALID.with(&[&slot, s]));
            }
            Some(_) => {}
        }
        if !(1..=4094).contains(&adapter.vlan) {
            errors.push(qvsmsg::VLAN_INVALID.with(&[&slot, &adapter.vlan.to_string()]));
        }
        for (i, t) in adapter.tagged.iter().enumerate() {
            let bad = !(1..=4094).contains(t)
                || *t == adapter.vlan
                || adapter.tagged[..i].contains(t)
                || i >= MAX_TAGGED;
            if bad {
                errors.push(qvsmsg::TAGGED_INVALID.with(&[&slot, &t.to_string()]));
            }
        }
        if let Some(mac) = adapter.mac.as_deref() {
            if !valid_mac(mac) {
                errors.push(qvsmsg::MAC_INVALID.with(&[&slot, mac]));
            } else if macs.contains(&mac.to_ascii_lowercase()) {
                let other = profile
                    .ethernet
                    .iter()
                    .find(|o| {
                        o.mac
                            .as_deref()
                            .is_some_and(|m| m.eq_ignore_ascii_case(mac))
                    })
                    .map_or(0, |o| o.slot);
                errors.push(qvsmsg::MAC_IN_USE.with(&[
                    &slot,
                    mac,
                    &format!("the adapter in slot {other}"),
                ]));
            }
            macs.push(mac.to_ascii_lowercase());
        }
    }
}

/// The sections later phases bring: checked for shape only.
fn later(profile: &Profile, errors: &mut Vec<Issued>) {
    for (kind, devices) in [("optical", &profile.optical), ("tape", &profile.tape)] {
        for device in devices {
            if let Some(media) = device.media.as_deref().filter(|m| !valid_store_name(m)) {
                errors.push(qvsmsg::MEDIA_INVALID.with(&[kind, &device.slot.to_string(), media]));
            }
        }
    }
    if profile.boot.source == crate::profile::IplSource::D && profile.optical.is_empty() {
        errors.push(qvsmsg::NO_OPTICAL.with(&[]));
    }
    for (i, device) in profile.physical_io.iter().enumerate() {
        let location = &device.location;
        let shaped = (1..=64).contains(&location.len())
            && location
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
        if !shaped {
            errors.push(qvsmsg::LOCATION_INVALID.with(&[location]));
        } else if profile.physical_io[..i]
            .iter()
            .any(|d| d.location == *location)
        {
            errors.push(qvsmsg::LOCATION_TWICE.with(&[location]));
        }
    }
}

/// Every error of a profile.
pub fn profile(profile: &Profile, limits: Limits) -> Vec<Issued> {
    let mut errors = Vec::new();
    if !valid_name(&profile.name) {
        errors.push(qvsmsg::PROFILE_NAME_INVALID.with(&[&profile.name]));
    }
    processors(&profile.processors, &mut errors);
    memory(profile, limits, &mut errors);
    slots(profile, &mut errors);
    storage(profile, &mut errors);
    ethernet(profile, &mut errors);
    later(profile, &mut errors);
    errors
}

/// The first section of `profile` that a later phase brings, as the
/// message an activation refuses it with; `None` when it uses none.
pub fn not_yet(profile: &Profile) -> Option<Issued> {
    let refuse = |what: &str, phase: u32| qvsmsg::NOT_YET.with(&[what, &phase.to_string()]);
    (!profile.physical_io.is_empty()).then(|| refuse("Physical I/O", 10))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{
        Boot, EthernetAdapter, IplSource, Memory, PhysicalIo, ProcessorMode, Sharing,
    };
    use crate::sample;

    const LIMITS: Limits = Limits {
        memory_block_mib: 16,
    };

    fn ids(errors: &[Issued]) -> Vec<&str> {
        errors.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn the_samples_are_valid() {
        assert_eq!(profile(&sample::dedicated(), LIMITS), []);
        assert_eq!(profile(&sample::shared(), LIMITS), []);
        assert_eq!(definition(&sample::definition()), []);
        assert!(not_yet(&sample::shared()).is_none());
    }

    #[test]
    fn names_follow_the_rule() {
        for good in ["A", "PROD", "TEST_01", "Z234567890"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "prod", "1PROD", "_A", "A-B", "A B", "ABCDEFGHIJK", "É"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn every_error_of_a_bad_profile_is_given_at_once() {
        let mut bad = sample::shared();
        bad.name = "bad name".into();
        bad.processors.minimum = 3;
        bad.processors.desired = 2;
        bad.processors.minimum_units = Some(Units(10));
        bad.processors.weight = Some(300);
        bad.processors.pool = Some("no".into());
        bad.memory = Memory {
            minimum_mib: 1000,
            desired_mib: 512,
            maximum_mib: 64,
        };
        bad.scsi[0].luns[0].backing = "relative".into();
        bad.scsi[0].slot = 1;
        let errors = profile(&bad, LIMITS);
        let got = ids(&errors);
        for want in [
            &qvsmsg::PROFILE_NAME_INVALID,
            &qvsmsg::PROCESSORS_ORDER,
            &qvsmsg::UNITS_TOO_FEW,
            &qvsmsg::WEIGHT_INVALID,
            &qvsmsg::POOL_NAME_INVALID,
            &qvsmsg::MEMORY_ORDER,
            &qvsmsg::MEMORY_BLOCK,
            &qvsmsg::MEMORY_RANGE,
            &qvsmsg::BACKING_INVALID,
            &qvsmsg::SLOT_INVALID,
        ] {
            assert!(got.contains(&want.id), "no {} in {got:?}", want.id);
        }
        // The minimum's units are too few for its three virtual
        // processors, and the message says so with the numbers.
        let few = errors
            .iter()
            .find(|e| e.is(&qvsmsg::UNITS_TOO_FEW))
            .unwrap();
        assert!(
            few.text.contains("0.10") && few.text.contains("0.15"),
            "{few}"
        );
    }

    #[test]
    fn units_are_between_a_twentieth_and_one_for_each_virtual_processor() {
        let mut p = sample::shared();
        p.processors.minimum = 1;
        p.processors.desired = 2;
        p.processors.maximum = 4;
        p.processors.minimum_units = Some(Units(5));
        p.processors.desired_units = Some(Units(200));
        p.processors.maximum_units = Some(Units(400));
        assert_eq!(profile(&p, LIMITS), []);
        p.processors.desired_units = Some(Units(201));
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::UNITS_TOO_MANY.id]);
        p.processors.desired_units = Some(Units(9));
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::UNITS_TOO_FEW.id]);
        p.processors.desired_units = None;
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::UNITS_MISSING.id]);
    }

    #[test]
    fn a_dedicated_profile_has_nothing_of_a_shared_one() {
        let mut p = sample::dedicated();
        p.processors.sharing = Some(Sharing::Uncapped);
        p.processors.desired_units = Some(Units(100));
        let errors = profile(&p, LIMITS);
        assert_eq!(ids(&errors), [qvsmsg::SHARED_ONLY.id; 2]);
        assert!(errors[0].text.contains("desired_units"));
        p.processors.mode = ProcessorMode::Shared;
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::UNITS_MISSING.id]);
    }

    #[test]
    fn slots_units_and_backing_stores_are_one_each() {
        let mut p = sample::dedicated();
        let mut second = p.scsi[0].clone();
        second.luns[0].serial = None;
        p.scsi.push(second);
        let got = profile(&p, LIMITS);
        assert_eq!(
            ids(&got),
            [
                qvsmsg::SLOT_TWICE.id,
                qvsmsg::LUN_TWICE.id,
                qvsmsg::BACKING_TWICE.id
            ]
        );
        p.scsi[1].slot = 3;
        p.scsi[1].luns[0].backing = "/dev/vdc".into();
        p.scsi[1].luns[0].serial = p.scsi[0].luns[0].serial.clone();
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::UNIT_SERIAL_TWICE.id]);
        p.scsi[1].luns[0].serial = Some("no spaces".into());
        p.scsi[1].luns[0].lun = 64;
        assert_eq!(
            ids(&profile(&p, LIMITS)),
            [
                qvsmsg::LUN_GAP.id,
                qvsmsg::LUN_INVALID.id,
                qvsmsg::UNIT_SERIAL_INVALID.id
            ]
        );
        p.scsi.clear();
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::NO_LOAD_SOURCE.id]);
        // The units are in the order of slot and unit number.
        let p = sample::shared();
        assert_eq!(p.units()[0].1.lun, 0);
    }

    #[test]
    fn the_sections_of_later_phases_are_checked_for_shape_and_refused_at_activation() {
        let mut p = sample::dedicated();
        p.ethernet.push(EthernetAdapter {
            slot: 4,
            switch: Some("PRODNET".into()),
            vlan: 1,
            tagged: vec![20],
            mac: Some("02:00:00:00:00:01".into()),
            required: true,
        });
        p.optical.push(MediaDevice {
            slot: 5,
            media: None,
        });
        p.physical_io.push(PhysicalIo {
            location: "P0000-B00-D05-F0".into(),
            required: true,
        });
        assert_eq!(profile(&p, LIMITS), []);
        // Optical and tape devices, IPL source D and Ethernet adapters
        // are taken; physical I/O is Phase 10's.
        let refused = not_yet(&p).unwrap();
        assert!(refused.is(&qvsmsg::NOT_YET));
        assert!(refused.text.contains("Phase 10"));
        p.physical_io.clear();
        p.boot = Boot {
            source: IplSource::D,
            ..Boot::default()
        };
        assert_eq!(profile(&p, LIMITS), []);
        assert_eq!(not_yet(&p), None);
        // Source D with no optical device is refused.
        p.optical.clear();
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::NO_OPTICAL.id]);

        let mut p = sample::dedicated();
        p.ethernet.push(EthernetAdapter {
            slot: 2,
            switch: None,
            vlan: 4095,
            tagged: vec![],
            mac: Some("01:00:5e:00:00:01".into()),
            required: true,
        });
        p.tape.push(MediaDevice {
            slot: 9,
            media: Some("/tmp/tape.img".into()),
        });
        p.physical_io.push(PhysicalIo {
            location: "a b".into(),
            required: false,
        });
        assert_eq!(
            ids(&profile(&p, LIMITS)),
            [
                qvsmsg::SLOT_TWICE.id,
                qvsmsg::ADAPTER_SWITCH_NEEDED.id,
                qvsmsg::VLAN_INVALID.id,
                qvsmsg::MAC_INVALID.id,
                qvsmsg::MEDIA_INVALID.id,
                qvsmsg::LOCATION_INVALID.id
            ]
        );
    }

    #[test]
    fn an_ethernet_adapters_errors_are_all_given() {
        let adapter =
            |slot, switch: &str, vlan, tagged: &[u32], mac: Option<&str>| EthernetAdapter {
                slot,
                switch: Some(switch.into()),
                vlan,
                tagged: tagged.to_vec(),
                mac: mac.map(str::to_string),
                required: false,
            };
        let mut p = sample::dedicated();
        p.ethernet = vec![
            adapter(4, "prod", 10, &[10, 20, 20, 0], Some("02:00:00:00:00:01")),
            adapter(5, "PRODNET", 10, &[], Some("02:00:00:00:00:01")),
        ];
        assert_eq!(
            ids(&profile(&p, LIMITS)),
            [
                qvsmsg::ADAPTER_SWITCH_INVALID.id,
                qvsmsg::TAGGED_INVALID.id,
                qvsmsg::TAGGED_INVALID.id,
                qvsmsg::TAGGED_INVALID.id,
                qvsmsg::MAC_IN_USE.id,
            ]
        );
        p.ethernet = (2..=10)
            .map(|slot| adapter(slot + 10, "PRODNET", 10, &[], None))
            .collect();
        assert_eq!(ids(&profile(&p, LIMITS)), [qvsmsg::ETHERNET_TOO_MANY.id]);
        // As a state file: required unless it says otherwise, and a
        // profile of Phase 5 without its switch is refused, not misread.
        let text = "slot = 4\nswitch = \"PRODNET\"\nvlan = 10\ntagged = [20]\n";
        let a: EthernetAdapter = toml::from_str(text).unwrap();
        assert!(a.required);
        assert_eq!(a.tagged_vlans(), [20]);
        assert_eq!(toml::to_string(&a).unwrap(), text);
        let old: EthernetAdapter = toml::from_str("slot = 4\nvlan = 10\n").unwrap();
        p.ethernet = vec![old];
        assert_eq!(
            ids(&profile(&p, LIMITS)),
            [qvsmsg::ADAPTER_SWITCH_NEEDED.id]
        );
    }

    #[test]
    fn a_definition_s_errors_are_all_given() {
        let mut d = sample::definition();
        d.id = 0;
        d.name = "x".into();
        d.serial = "short".into();
        d.default_profile = "also bad".into();
        d.auto_start.delay_secs = 7200;
        assert_eq!(
            ids(&definition(&d)),
            [
                qvsmsg::ID_INVALID.id,
                qvsmsg::NAME_INVALID.id,
                qvsmsg::SERIAL_INVALID.id,
                qvsmsg::PROFILE_NAME_INVALID.id,
                qvsmsg::DELAY_INVALID.id
            ]
        );
    }
}
