// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A profile's physical I/O against the host's devices: what a put is
//! refused for, and what an activation is given. A device goes with
//! every other device of its isolation group, so a profile names a
//! group whole or not at all. A required device that cannot be given
//! fails the activation by name; one that is not required is left out,
//! with its group, and the partition activates without it.

use model::profile::PhysicalIo;
use qvsmsg::Issued;

use crate::device::{Blocker, Device};

/// What an activation is given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Assignment {
    /// The devices given, every group whole, by location code.
    pub given: Vec<String>,
    /// Why each device that is not required was left out.
    pub left_out: Vec<Issued>,
}

/// What a profile's put is refused for: a location no device of the
/// host has, a location twice, part of a group named without the rest.
/// Whether a device is released, free or fenced is the activation's.
pub fn check_profile(devices: &[Device], wanted: &[PhysicalIo]) -> Vec<Issued> {
    let mut errors = Vec::new();
    for (i, w) in wanted.iter().enumerate() {
        if wanted[..i].iter().any(|o| o.location == w.location) {
            continue;
        }
        match devices.iter().find(|d| d.location == w.location) {
            None => errors.push(qvsmsg::DEVICE_UNKNOWN.with(&[&w.location])),
            Some(d) => errors.extend(split(d, wanted)),
        }
    }
    errors
}

/// The refusal for `d` named without the rest of its group.
fn split(d: &Device, wanted: &[PhysicalIo]) -> Option<Issued> {
    let missing: Vec<&str> = d
        .with
        .iter()
        .filter(|o| !wanted.iter().any(|w| w.location == **o))
        .map(String::as_str)
        .collect();
    (!missing.is_empty())
        .then(|| qvsmsg::DEVICE_GROUP_SPLIT.with(&[&d.location, &missing.join(", ")]))
}

/// Why `d` cannot be given to `partition` now; none when it can be.
fn refusal(d: Option<&Device>, location: &str, partition: u32) -> Option<Issued> {
    let Some(d) = d else {
        return Some(qvsmsg::DEVICE_UNKNOWN.with(&[location]));
    };
    if let Some(h) = &d.holder
        && h.partition != partition
    {
        return Some(qvsmsg::DEVICE_HELD.with(&[location, &h.name]));
    }
    match &d.blocked {
        None => None,
        Some((Blocker::NotReleased, _)) => Some(qvsmsg::DEVICE_NOT_RELEASED.with(&[location])),
        Some((_, why)) => Some(qvsmsg::DEVICE_NOT_ASSIGNABLE.with(&[location, &why.text])),
    }
}

/// What an activation of `partition` (`name`) is given of `wanted`
/// among `devices`, or every reason it fails.
pub fn assignment(
    devices: &[Device],
    wanted: &[PhysicalIo],
    partition: u32,
    name: &str,
) -> Result<Assignment, Vec<Issued>> {
    let mut errors = Vec::new();
    let mut out = Assignment::default();
    let mut done: Vec<&str> = Vec::new();
    for w in wanted {
        if done.contains(&w.location.as_str()) {
            continue;
        }
        let d = devices.iter().find(|d| d.location == w.location);
        // The group as the profile names it: every one of its devices.
        let group: Vec<String> =
            d.map_or_else(|| vec![w.location.clone()], Device::group_locations);
        if let Some(d) = d
            && let Some(e) = split(d, wanted)
        {
            errors.push(e);
            done.extend(group.iter().filter_map(|l| {
                wanted
                    .iter()
                    .find(|w| w.location == *l)
                    .map(|w| w.location.as_str())
            }));
            continue;
        }
        let entries: Vec<&PhysicalIo> = group
            .iter()
            .filter_map(|l| wanted.iter().find(|w| w.location == *l))
            .collect();
        let required = entries.iter().any(|e| e.required);
        let reasons: Vec<Issued> = group
            .iter()
            .filter_map(|l| refusal(devices.iter().find(|d| d.location == *l), l, partition))
            .collect();
        done.extend(entries.iter().map(|e| e.location.as_str()));
        if reasons.is_empty() {
            out.given.extend(group);
        } else if required {
            errors.extend(reasons);
        } else {
            out.left_out.extend(
                reasons
                    .iter()
                    .map(|r| qvsmsg::DEVICE_LEFT_OUT.with(&[&w.location, name, &r.text])),
            );
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Holder;
    use crate::device::tests::{ON, devices, host};
    use crate::released::{Released, ReleasedDevice};

    fn want(location: &str, required: bool) -> PhysicalIo {
        PhysicalIo {
            location: location.into(),
            required,
        }
    }

    const NIC: &str = "P0000-B00-D05-F0";
    const NVME: &str = "P0000-B00-D06-F0";
    const F0: &str = "P0000-B00-D07-F0";
    const F1: &str = "P0000-B00-D07-F1";

    fn released_all() -> Vec<Device> {
        let pci = host();
        let pair = vec![F0.to_string(), F1.to_string()];
        let r = Released {
            devices: vec![
                ReleasedDevice::of(&pci[4], &[]),
                ReleasedDevice::of(&pci[5], &[]),
                ReleasedDevice::of(&pci[6], &pair),
                ReleasedDevice::of(&pci[7], &pair),
            ],
            drives: Vec::new(),
        };
        devices(&pci, &ON, &r)
    }

    #[test]
    fn a_put_names_known_devices_and_whole_groups() {
        let d = released_all();
        assert!(check_profile(&d, &[want(NIC, true), want(F0, false), want(F1, false)]).is_empty());
        let e = check_profile(&d, &[want("P0000-B00-D09-F0", true), want(F0, true)]);
        assert!(e[0].is(&qvsmsg::DEVICE_UNKNOWN));
        assert!(
            e[1].is(&qvsmsg::DEVICE_GROUP_SPLIT) && e[1].text.contains(F1),
            "{e:?}"
        );
    }

    #[test]
    fn an_activation_gets_whole_groups() {
        let d = released_all();
        let a = assignment(
            &d,
            &[want(F0, true), want(F1, true), want(NVME, false)],
            2,
            "PROD",
        )
        .unwrap();
        assert_eq!(a.given, [F0, F1, NVME]);
        assert!(a.left_out.is_empty());
        let e = assignment(&d, &[want(F1, true)], 2, "PROD").unwrap_err();
        assert!(
            e[0].is(&qvsmsg::DEVICE_GROUP_SPLIT) && e[0].text.contains(F0),
            "{e:?}"
        );
    }

    #[test]
    fn held_required_fails_by_name_and_held_not_required_is_left_out() {
        let mut d = released_all();
        d.iter_mut().find(|d| d.location == NVME).unwrap().holder = Some(Holder {
            partition: 1,
            name: "FIRST".into(),
            profile: "NORMAL".into(),
        });
        let e = assignment(&d, &[want(NVME, true)], 2, "SECOND").unwrap_err();
        assert!(
            e[0].is(&qvsmsg::DEVICE_HELD) && e[0].text.contains("FIRST"),
            "{e:?}"
        );
        let a = assignment(&d, &[want(NVME, false), want(NIC, true)], 2, "SECOND").unwrap();
        assert_eq!(a.given, [NIC]);
        assert!(a.left_out[0].is(&qvsmsg::DEVICE_LEFT_OUT));
        assert!(a.left_out[0].text.contains("FIRST"), "{:?}", a.left_out);
        // The holder itself is not refused its own device.
        assert!(assignment(&d, &[want(NVME, true)], 1, "FIRST").is_ok());
    }

    #[test]
    fn not_released_unknown_and_fenced_are_refused_by_name() {
        let pci = host();
        let d = devices(&pci, &ON, &Released::default());
        let e = assignment(
            &d,
            &[want(NIC, true), want("P0000-B00-D0A-F0", true)],
            2,
            "P",
        )
        .unwrap_err();
        assert!(e[0].is(&qvsmsg::DEVICE_NOT_RELEASED) && e[0].text.contains(NIC));
        assert!(e[1].is(&qvsmsg::DEVICE_UNKNOWN));
        let mut d = released_all();
        let nvme = d.iter_mut().find(|d| d.location == NVME).unwrap();
        nvme.fenced = true;
        nvme.blocked = Some((Blocker::Fenced, qvsmsg::DEVICE_FENCED.with(&[NVME])));
        let e = assignment(&d, &[want(NVME, true)], 2, "P").unwrap_err();
        assert!(e[0].is(&qvsmsg::DEVICE_NOT_ASSIGNABLE), "{e:?}");
        assert!(e[0].text.contains("fenced"), "{e:?}");
        // The host's own device: refused with the reason it is the host's.
        let e = assignment(&d, &[want("P0000-B00-D03-F0", true)], 2, "P").unwrap_err();
        assert!(e[0].is(&qvsmsg::DEVICE_NOT_ASSIGNABLE) && e[0].text.contains("management"));
    }
}
