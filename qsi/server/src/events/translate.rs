// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The services' events as QSI's (`docs/QSI.md` section 8): the
//! partition manager's and the switch service's as they come; the pool
//! service's, which it does not send, from what changed between two
//! looks.

use std::collections::BTreeMap;

use qsi::any::AnyEvent;
use qsi::host::{dumps, partitions, profiles, refcodes, vres};

use crate::families::convert;
use crate::families::vres as v;

/// Whether a posted code is a note of the history and not the current
/// code: the host's `C2009` codes and "found running".
fn is_note(p: &partd::Posted) -> bool {
    p.source == partd::proto::Source::Host
        && (p.code.starts_with("C2009") || p.code == refcode::PARTITION_ADOPTED.code)
}

/// A partition manager's event.
#[must_use]
pub fn partd(e: &partd::Event) -> AnyEvent {
    use partd::Event as E;
    match e {
        E::State {
            id,
            name,
            state,
            time,
        } => AnyEvent::Partitions(partitions::Event::State(partitions::StateChanged {
            id: *id,
            name: name.clone(),
            state: convert::state(*state),
            time: *time,
        })),
        E::Code { id, name, posted } => AnyEvent::Refcodes(refcodes::Event::Code(refcodes::Code {
            id: *id,
            name: name.clone(),
            posted: convert::posted(posted, 0),
            current: !is_note(posted),
        })),
        E::Resources {
            id,
            name,
            current,
            time,
        } => AnyEvent::Partitions(partitions::Event::Resources(partitions::ResourcesChanged {
            id: *id,
            name: name.clone(),
            current: current.as_ref().map(convert::current),
            time: *time,
        })),
        E::Defined {
            id,
            name,
            exists,
            time,
        } => AnyEvent::Partitions(partitions::Event::Defined(partitions::Defined {
            id: *id,
            name: name.clone(),
            exists: *exists,
            time: *time,
        })),
        E::Channel {
            id,
            name,
            service,
            time,
        } => AnyEvent::Partitions(partitions::Event::Channel(partitions::ChannelChanged {
            id: *id,
            name: name.clone(),
            channel: convert::channel(service),
            time: *time,
        })),
        E::Dump {
            id,
            name,
            dump,
            error,
            time,
        } => AnyEvent::Dumps(dumps::Event::Taken(dumps::Taken {
            id: *id,
            name: name.clone(),
            dump: dump.clone(),
            error: error.as_ref().map(convert::note),
            time: *time,
        })),
        E::Profile {
            id,
            profile,
            revision,
            default,
            time,
            ..
        } => AnyEvent::Profiles(profiles::Event::Changed(profiles::Changed {
            id: *id,
            name: profile.clone(),
            revision: *revision,
            default: *default,
            time: *time,
        })),
    }
}

/// Whether a partition manager's event changes the host's resources.
#[must_use]
pub fn moves_resources(e: &partd::Event) -> bool {
    matches!(e, partd::Event::Resources { .. })
}

/// A switch service's event.
#[must_use]
pub fn vswitchd(e: &vswitchd::Event) -> AnyEvent {
    use vswitchd::Event as E;
    AnyEvent::Vres(match e {
        E::Switch { name, status, time } => vres::Event::Switch(vres::SwitchChanged {
            name: name.clone(),
            switch: status.as_ref().and_then(v::switch),
            time: *time,
        }),
        E::Uplink {
            switch,
            location,
            carrier,
            state,
            time,
        } => vres::Event::Uplink(vres::UplinkChanged {
            switch: switch.clone(),
            location: location.clone(),
            carrier: *carrier,
            state: state.clone(),
            time: *time,
        }),
        E::Port {
            partition,
            slot,
            switch,
            open,
            time,
        } => vres::Event::Port(vres::PortChanged {
            partition: *partition,
            slot: *slot,
            switch: switch.clone(),
            open: *open,
            time: *time,
        }),
    })
}

/// What the pool service has, at one look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// Each pool, by name.
    pub pools: BTreeMap<String, vres::StoragePool>,
    /// Each virtual disk, by pool and name.
    pub vdisks: BTreeMap<(String, String), vres::Vdisk>,
    /// Each optical image, by name.
    pub images: BTreeMap<String, vres::Image>,
    /// Each tape volume, by name.
    pub volumes: BTreeMap<String, vres::Volume>,
}

impl Look {
    /// A look from the pool service's lists.
    #[must_use]
    pub fn of(
        pools: &[poold::PoolStatus],
        vdisks: &[poold::Vdisk],
        images: &[poold::Image],
        volumes: &[poold::Volume],
    ) -> Self {
        Self {
            pools: pools.iter().map(|p| (p.name.clone(), v::pool(p))).collect(),
            vdisks: vdisks
                .iter()
                .map(|d| ((d.pool.clone(), d.name.clone()), v::vdisk(d)))
                .collect(),
            images: images
                .iter()
                .map(|i| (i.name.clone(), v::image(i)))
                .collect(),
            volumes: volumes
                .iter()
                .map(|o| (o.name.clone(), v::volume(o)))
                .collect(),
        }
    }
}

/// Every key of two maps whose values differ, with the new value or
/// none when gone.
fn changed<'a, K: Ord + Clone, V: PartialEq + Clone>(
    before: &'a BTreeMap<K, V>,
    after: &'a BTreeMap<K, V>,
) -> Vec<(K, Option<V>)> {
    let mut out = Vec::new();
    for (k, a) in after {
        if before.get(k) != Some(a) {
            out.push((k.clone(), Some(a.clone())));
        }
    }
    for k in before.keys() {
        if !after.contains_key(k) {
            out.push((k.clone(), None));
        }
    }
    out
}

/// The events of what changed between two looks at the pool service.
#[must_use]
pub fn pools(before: &Look, after: &Look, time: u64) -> Vec<AnyEvent> {
    let mut out = Vec::new();
    for (name, pool) in changed(&before.pools, &after.pools) {
        out.push(vres::Event::Pool(vres::PoolChanged { name, pool, time }));
    }
    for ((pool, name), vdisk) in changed(&before.vdisks, &after.vdisks) {
        out.push(vres::Event::Vdisk(vres::VdiskChanged {
            pool,
            name,
            vdisk,
            time,
        }));
    }
    for (name, image) in changed(&before.images, &after.images) {
        out.push(vres::Event::Image(vres::ImageChanged { name, image, time }));
    }
    for (name, volume) in changed(&before.volumes, &after.volumes) {
        out.push(vres::Event::Volume(vres::VolumeChanged {
            name,
            volume,
            time,
        }));
    }
    out.into_iter().map(AnyEvent::Vres).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_of_the_history_is_not_the_current_code() {
        let code = |code: &str| partd::Event::Code {
            id: 1,
            name: "P1".into(),
            posted: partd::Posted {
                code: code.into(),
                ..partd::Posted::default()
            },
        };
        let current = |e: AnyEvent| match e {
            AnyEvent::Refcodes(refcodes::Event::Code(c)) => c.current,
            _ => panic!(),
        };
        assert!(current(partd(&code("C2005090"))));
        assert!(!current(partd(&code("C2009010"))));
    }

    #[test]
    fn what_changed_at_the_pool_service_is_said_once() {
        let image = |name: &str, bytes| poold::Image {
            name: name.into(),
            bytes,
            ..poold::Image::default()
        };
        let a = Look::of(&[], &[], &[image("CD1", 1), image("CD2", 2)], &[]);
        let b = Look::of(&[], &[], &[image("CD1", 1), image("CD3", 3)], &[]);
        let events = pools(&a, &b, 9);
        let said: Vec<(String, bool)> = events
            .iter()
            .map(|e| match e {
                AnyEvent::Vres(vres::Event::Image(i)) => (i.name.clone(), i.image.is_some()),
                _ => panic!("{e:?}"),
            })
            .collect();
        assert_eq!(said, vec![("CD3".into(), true), ("CD2".into(), false)]);
        assert!(pools(&b, &b, 10).is_empty());
    }
}
