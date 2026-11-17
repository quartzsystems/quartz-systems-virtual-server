// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A running shared partition's entitlement (`docs/RESOURCES.md`): its
//! processing units, capped or uncapped, and its weight, changed while
//! it runs, within its profile's minimum and maximum units, what its
//! virtual processors can use, and what its pool has. Its virtual
//! processors, a dedicated partition's processors and memory change
//! only at activation. The profile is not changed: the next activation
//! takes it again.

use model::Units;
use model::profile::{DEFAULT_WEIGHT, MAX_WEIGHT, Profile, Sharing};
use qvsmsg::Issued;

use crate::account::{Current, Held, Host, Processors};

/// What a change names; what is none stays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entitlement {
    /// The processing units.
    pub units: Option<Units>,
    /// Capped or uncapped.
    pub sharing: Option<Sharing>,
    /// The uncapped weight.
    pub weight: Option<u32>,
}
ipc::wire_struct!(Entitlement {
    units,
    sharing,
    weight
});

impl Host {
    /// What the partition `id` (`name`) of `held`, activated with
    /// `profile`, holds after `change`; every reason it cannot.
    pub fn entitle(
        &self,
        held: &[Held],
        id: u32,
        name: &str,
        profile: &Profile,
        change: Entitlement,
    ) -> Result<Current, Vec<Issued>> {
        let Some(current) = held.iter().find(|h| h.partition == id).map(|h| &h.current) else {
            return Err(vec![qvsmsg::ENTITLEMENT_NOT_RUNNING.with(&[name])]);
        };
        let Processors::Shared {
            pool,
            units,
            virtuals,
            capped,
            weight,
        } = &current.processors
        else {
            return Err(vec![qvsmsg::ENTITLEMENT_DEDICATED.with(&[name])]);
        };
        if change == Entitlement::default() {
            return Err(vec![
                qvsmsg::ENTITLEMENT_INVALID
                    .with(&["it names none of the units, capping and weight"]),
            ]);
        }
        if let Some(w) = change.weight.filter(|w| *w > MAX_WEIGHT) {
            return Err(vec![
                qvsmsg::ENTITLEMENT_INVALID.with(&[&format!("weight {w} is above {MAX_WEIGHT}")]),
            ]);
        }
        let mut errors = Vec::new();
        let new_units = change.units.unwrap_or(*units);
        if change.units.is_some() {
            let p = &profile.processors;
            let (least, most) = (
                p.minimum_units.unwrap_or_default(),
                p.maximum_units.unwrap_or_default(),
            );
            if new_units < least || new_units > most {
                errors.push(qvsmsg::ENTITLEMENT_OUTSIDE.with(&[
                    &new_units.to_string(),
                    name,
                    &least.to_string(),
                    &most.to_string(),
                ]));
            }
            let floor = Units(Units::LEAST_PER_VIRTUAL.0.saturating_mul(*virtuals));
            let worth = Units::cores(*virtuals);
            if new_units < floor || new_units > worth {
                errors.push(qvsmsg::ENTITLEMENT_VIRTUALS.with(&[
                    &new_units.to_string(),
                    name,
                    &virtuals.to_string(),
                    &floor.to_string(),
                    &worth.to_string(),
                ]));
            }
            // What it holds now is its own to keep.
            let others: Vec<Held> = held.iter().filter(|h| h.partition != id).cloned().collect();
            let room = self.pool(pool, &others).map_or(Units(0), |p| p.available);
            if new_units > room {
                errors.push(qvsmsg::ENTITLEMENT_POOL_SHORT.with(&[
                    pool,
                    &new_units.to_string(),
                    name,
                    &room.to_string(),
                ]));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let now_capped = match change.sharing {
            Some(Sharing::Capped) => true,
            Some(Sharing::Uncapped) => false,
            None => *capped,
        };
        let new_weight = if now_capped {
            0
        } else {
            change.weight.unwrap_or(if *capped {
                profile.processors.weight.unwrap_or(DEFAULT_WEIGHT)
            } else {
                *weight
            })
        };
        // An uncapped weight of 0 is a capped partition.
        let now_capped = now_capped || new_weight == 0;
        Ok(Current {
            processors: Processors::Shared {
                pool: pool.clone(),
                units: new_units,
                virtuals: *virtuals,
                capped: now_capped,
                weight: if now_capped { 0 } else { new_weight },
            },
            memory_mib: current.memory_mib,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::topology::Topology;
    use model::sample;

    fn host() -> Host {
        Host {
            topology: Topology::uniform(8, 2),
            installed_mib: 16_384,
            configurable_mib: 14_336,
            settings: Settings::default(),
        }
    }

    /// The sample shared profile with 0.20 to 2.00 units over two
    /// virtual processors, capped at 0.50.
    fn shared() -> (Profile, Current) {
        let mut p = sample::shared();
        p.processors.minimum = 2;
        p.processors.desired = 2;
        p.processors.maximum = 2;
        p.processors.minimum_units = Some(Units(20));
        p.processors.desired_units = Some(Units(50));
        p.processors.maximum_units = Some(Units(200));
        p.processors.sharing = Some(Sharing::Capped);
        let current = host().resolve(&[], "PROD", &p).unwrap();
        (p, current)
    }

    fn held(current: &Current) -> Vec<Held> {
        vec![Held {
            partition: 1,
            current: current.clone(),
        }]
    }

    #[test]
    fn units_rise_within_the_profile_and_the_rest_stays() {
        let (p, current) = shared();
        let change = Entitlement {
            units: Some(Units(100)),
            ..Entitlement::default()
        };
        let after = host()
            .entitle(&held(&current), 1, "PROD", &p, change)
            .unwrap();
        assert_eq!(after.processors.units(), Units(100));
        assert_eq!(after.processors.processors(), 2);
        assert_eq!(after.memory_mib, current.memory_mib);
        assert!(matches!(
            after.processors,
            Processors::Shared {
                capped: true,
                weight: 0,
                ..
            }
        ));
    }

    #[test]
    fn uncapping_takes_the_profile_s_weight_or_the_default() {
        let (mut p, current) = shared();
        let uncap = Entitlement {
            sharing: Some(Sharing::Uncapped),
            ..Entitlement::default()
        };
        let after = host()
            .entitle(&held(&current), 1, "PROD", &p, uncap)
            .unwrap();
        assert!(matches!(
            after.processors,
            Processors::Shared {
                capped: false,
                weight: 128,
                ..
            }
        ));
        p.processors.weight = Some(40);
        let after = host()
            .entitle(&held(&current), 1, "PROD", &p, uncap)
            .unwrap();
        assert!(matches!(
            after.processors,
            Processors::Shared {
                capped: false,
                weight: 40,
                ..
            }
        ));
        let both = Entitlement {
            sharing: Some(Sharing::Uncapped),
            weight: Some(200),
            ..Entitlement::default()
        };
        let after = host()
            .entitle(&held(&current), 1, "PROD", &p, both)
            .unwrap();
        assert!(matches!(
            after.processors,
            Processors::Shared { weight: 200, .. }
        ));
    }

    #[test]
    fn every_refusal_says_why() {
        let (p, current) = shared();
        let h = host();
        let refused = |change| {
            h.entitle(&held(&current), 1, "PROD", &p, change)
                .unwrap_err()
                .into_iter()
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };
        let units = |u| Entitlement {
            units: Some(Units(u)),
            ..Entitlement::default()
        };
        assert_eq!(
            refused(units(8)),
            [
                qvsmsg::ENTITLEMENT_OUTSIDE.id,
                qvsmsg::ENTITLEMENT_VIRTUALS.id
            ]
        );
        assert_eq!(
            refused(units(250)),
            [
                qvsmsg::ENTITLEMENT_OUTSIDE.id,
                qvsmsg::ENTITLEMENT_VIRTUALS.id
            ]
        );
        assert_eq!(
            refused(Entitlement::default()),
            [qvsmsg::ENTITLEMENT_INVALID.id]
        );
        assert_eq!(
            refused(Entitlement {
                weight: Some(256),
                ..Entitlement::default()
            }),
            [qvsmsg::ENTITLEMENT_INVALID.id]
        );
        // Another partition holds most of the pool.
        let other = Held {
            partition: 2,
            current: Current {
                processors: Processors::Shared {
                    pool: "DEFAULT".into(),
                    units: Units(600),
                    virtuals: 6,
                    capped: true,
                    weight: 0,
                },
                memory_mib: 1024,
            },
        };
        let mut both = held(&current);
        both.push(other);
        let short = h.entitle(&both, 1, "PROD", &p, units(150)).unwrap_err();
        assert!(short[0].is(&qvsmsg::ENTITLEMENT_POOL_SHORT), "{short:?}");
        // Its own 0.50 counts for it: 1.00 fits beside the 6.00.
        assert!(h.entitle(&both, 1, "PROD", &p, units(100)).is_ok());
        // A dedicated partition is refused.
        let dedicated = Current {
            processors: Processors::Dedicated { cores: vec![7] },
            memory_mib: 1024,
        };
        let refused = h
            .entitle(
                &[Held {
                    partition: 1,
                    current: dedicated,
                }],
                1,
                "PROD",
                &p,
                units(100),
            )
            .unwrap_err();
        assert!(refused[0].is(&qvsmsg::ENTITLEMENT_DEDICATED));
        let refused = h.entitle(&[], 1, "PROD", &p, units(100)).unwrap_err();
        assert!(refused[0].is(&qvsmsg::ENTITLEMENT_NOT_RUNNING));
    }

    #[test]
    fn the_plan_writes_the_quota_and_weight_alone() {
        let (p, current) = shared();
        let h = host();
        let after = h
            .entitle(
                &held(&current),
                1,
                "PROD",
                &p,
                Entitlement {
                    units: Some(Units(100)),
                    ..Entitlement::default()
                },
            )
            .unwrap();
        let plan = crate::cgroup::entitlement(&h, 1, &after);
        assert_eq!(
            plan,
            [
                crate::cgroup::Step::Write(
                    "qsvs/pools/DEFAULT/p1/cpu.max".into(),
                    "200000 100000".into()
                ),
                crate::cgroup::Step::Write("qsvs/pools/DEFAULT/p1/cpu.weight".into(), "100".into()),
            ]
        );
    }
}
