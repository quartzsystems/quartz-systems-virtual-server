// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `physio.toml`: what an operator released for partitions at the host.
//! A device is kept with what it was when it was released, its IDs and
//! its group, so that at the IPL one that is gone or whose group
//! changed is not given to the pass-through driver. Physical tape drives
//! released for pass-through are kept here too.
//!
//! ```toml
//! schema_version = 1
//!
//! [[devices]]
//! location = "P0000-B00-D06-F0"
//! address = "0000:00:06.0"
//! vendor = "1b36"
//! device = "0010"
//! group = ["P0000-B00-D06-F0"]
//!
//! [[drives]]
//! location = "P0000-B00-D08-F0-T0.0.3.0"
//! serial = "HU1234567"
//! ```

use serde::{Deserialize, Serialize};
use state::doc::inventory::Pci;
use state::store::{Document, Migration};

use crate::device::is_bridge;

/// One released device.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleasedDevice {
    /// Its location code.
    pub location: String,
    /// Its PCI address when it was released.
    pub address: String,
    /// Its vendor ID.
    pub vendor: String,
    /// Its device ID.
    pub device: String,
    /// Its isolation group's devices then, it among them, bridges left
    /// out, by location code in order.
    #[serde(default)]
    pub group: Vec<String>,
}

impl ReleasedDevice {
    /// `p` as released with `others`, the rest of its group.
    pub fn of(p: &Pci, others: &[String]) -> Self {
        let mut group: Vec<String> = std::iter::once(p.location.clone())
            .chain(others.iter().cloned())
            .collect();
        group.sort();
        group.dedup();
        Self {
            location: p.location.clone(),
            address: p.address.clone(),
            vendor: p.vendor.clone(),
            device: p.device.clone(),
            group,
        }
    }
}

/// One physical tape drive released for pass-through.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleasedDrive {
    /// Its location code.
    pub location: String,
    /// Its serial number when it was released.
    #[serde(default)]
    pub serial: String,
}

/// `physio.toml`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Released {
    /// The released PCI devices, by location code.
    #[serde(default)]
    pub devices: Vec<ReleasedDevice>,
    /// The released physical tape drives, by location code.
    #[serde(default)]
    pub drives: Vec<ReleasedDrive>,
}

impl Document for Released {
    const FILE: &'static str = "physio.toml";
    const VERSION: u32 = 1;
    const MIGRATIONS: &'static [Migration] = &[];

    fn validate(&self) -> Result<(), String> {
        let mut seen: Vec<&str> = Vec::new();
        for l in self
            .devices
            .iter()
            .map(|d| d.location.as_str())
            .chain(self.drives.iter().map(|d| d.location.as_str()))
        {
            if l.is_empty() {
                return Err("a released device has no location code".into());
            }
            if seen.contains(&l) {
                return Err(format!("{l} is released twice"));
            }
            seen.push(l);
        }
        Ok(())
    }
}

/// What PID 1 does with a released device at the IPL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtIpl {
    /// Give the function at this address to the pass-through driver.
    Bind {
        /// The location code.
        location: String,
        /// The PCI address now.
        address: String,
    },
    /// Nothing is at the location, or something else is.
    Gone {
        /// The location code.
        location: String,
    },
    /// Its group is not what it was: what changed.
    Changed {
        /// The location code.
        location: String,
        /// What changed, as a person reads it.
        what: String,
    },
}

impl Released {
    /// Whether the device at `location` is released.
    pub fn has(&self, location: &str) -> bool {
        self.devices.iter().any(|d| d.location == location)
    }

    /// Whether the drive at `location` is released.
    pub fn has_drive(&self, location: &str) -> bool {
        self.drives.iter().any(|d| d.location == location)
    }

    /// How the group of `location` differs now from when it was
    /// released, given its group now (it among them); none when it does
    /// not or it is not released.
    pub fn group_change(&self, location: &str, now: &[String]) -> Option<String> {
        let was = &self.devices.iter().find(|d| d.location == location)?.group;
        if was.is_empty() {
            return None;
        }
        let mut now: Vec<String> = now.to_vec();
        now.sort();
        now.dedup();
        if *was == now {
            return None;
        }
        let gained: Vec<&str> = now
            .iter()
            .filter(|l| !was.contains(l))
            .map(String::as_str)
            .collect();
        let lost: Vec<&str> = was
            .iter()
            .filter(|l| !now.contains(l))
            .map(String::as_str)
            .collect();
        let mut what = Vec::new();
        if !gained.is_empty() {
            what.push(format!("it now has {}", gained.join(", ")));
        }
        if !lost.is_empty() {
            what.push(format!("it no longer has {}", lost.join(", ")));
        }
        Some(what.join("; "))
    }

    /// What each released device is at an IPL on `pci`: bound, gone or
    /// changed. A device whose vendor or device ID differs at its
    /// location is gone: something else is there.
    pub fn at_ipl(&self, pci: &[Pci]) -> Vec<AtIpl> {
        self.devices
            .iter()
            .map(|r| {
                let Some(p) = pci.iter().find(|p| {
                    p.location == r.location && p.vendor == r.vendor && p.device == r.device
                }) else {
                    return AtIpl::Gone {
                        location: r.location.clone(),
                    };
                };
                let now: Vec<String> = match p.group {
                    Some(g) => pci
                        .iter()
                        .filter(|o| o.group == Some(g) && !is_bridge(&o.class))
                        .map(|o| o.location.clone())
                        .collect(),
                    None => Vec::new(),
                };
                match self.group_change(&r.location, &now) {
                    Some(what) => AtIpl::Changed {
                        location: r.location.clone(),
                        what,
                    },
                    None if p.group.is_none() => AtIpl::Changed {
                        location: r.location.clone(),
                        what: "it is in no isolation group".into(),
                    },
                    None => AtIpl::Bind {
                        location: r.location.clone(),
                        address: p.address.clone(),
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::host;
    use state::store::{parse, render};

    fn released(pci: &[Pci], at: &[usize]) -> Released {
        let group: Vec<String> = at.iter().map(|i| pci[*i].location.clone()).collect();
        Released {
            devices: at
                .iter()
                .map(|i| ReleasedDevice::of(&pci[*i], &group))
                .collect(),
            drives: Vec::new(),
        }
    }

    #[test]
    fn the_file_reads_back_and_refuses_a_device_twice() {
        let pci = host();
        let mut r = released(&pci, &[6, 7]);
        r.drives.push(ReleasedDrive {
            location: "P0000-B00-D08-F0-T0.0.3.0".into(),
            serial: "HU1".into(),
        });
        let text = render(&r).unwrap();
        assert_eq!(parse::<Released>(&text).unwrap(), r);
        assert_eq!(r.devices[0].group, ["P0000-B00-D07-F0", "P0000-B00-D07-F1"]);
        r.devices.push(r.devices[0].clone());
        assert!(r.validate().unwrap_err().contains("twice"));
    }

    #[test]
    fn at_the_ipl_a_device_is_bound_gone_or_changed() {
        let pci = host();
        let r = released(&pci, &[5]);
        assert_eq!(
            r.at_ipl(&pci),
            [AtIpl::Bind {
                location: "P0000-B00-D06-F0".into(),
                address: "0000:00:06.0".into()
            }]
        );
        // Removed from the machine.
        let fewer: Vec<Pci> = pci.iter().filter(|p| p.group != Some(6)).cloned().collect();
        assert!(matches!(r.at_ipl(&fewer)[0], AtIpl::Gone { .. }));
        // Another device at its location.
        let mut other = pci.clone();
        other[5].device = "abcd".into();
        assert!(matches!(r.at_ipl(&other)[0], AtIpl::Gone { .. }));
        // A NIC moved into its group.
        let mut joined = pci.clone();
        joined[4].group = Some(6);
        match &r.at_ipl(&joined)[0] {
            AtIpl::Changed { what, .. } => assert!(what.contains("P0000-B00-D05-F0"), "{what}"),
            other => panic!("{other:?}"),
        }
        // The pair: one function gone.
        let r = released(&pci, &[6, 7]);
        let split: Vec<Pci> = pci
            .iter()
            .filter(|p| p.address != "0000:00:07.1")
            .cloned()
            .collect();
        let at = r.at_ipl(&split);
        assert!(matches!(&at[0], AtIpl::Changed { what, .. } if what.contains("no longer")));
        assert!(matches!(at[1], AtIpl::Gone { .. }));
    }
}
