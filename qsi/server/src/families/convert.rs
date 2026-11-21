// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's partition model to QSI's and back, field for field
//! (`docs/QSI.md` section 7, shared types). A value of an open QSI
//! enumeration this host does not have is refused `QSI0361`.

use model::Units;
use qsi::base::{Keylock as QKeylock, Refcode};
use qsi::host::model as q;
use qsi::host::partition::msgid::NO_SUCH_VALUE;
use qsisess::Note;

/// A value this host has no word for.
pub fn no_such(what: &str) -> Note {
    Note {
        msgid: NO_SUCH_VALUE.into(),
        text: format!("This host has no such {what}."),
        help: String::new(),
    }
}

/// Every message of a QS/VS refusal, as QSI notes.
pub fn notes(issued: &[qvsmsg::Issued]) -> Vec<Note> {
    issued
        .iter()
        .map(|i| Note {
            msgid: i.id.clone(),
            text: i.text.clone(),
            help: i.help.clone(),
        })
        .collect()
}

/// QSI's identifier for a refusal of physical I/O (`QSI1301` to
/// `QSI1307`) or of a running partition's entitlement (`QSI1401` to
/// `QSI1405`), from the host's message; none for any other.
pub fn qsi_identifier(id: &str) -> Option<&'static str> {
    use qsi::host::host::physio as p;
    use qsi::host::partition::entitlement as e;
    Some(match id {
        "QVS2401" => p::UNKNOWN_LOCATION,
        "QVS2402" => p::NOT_RELEASED,
        "QVS2403" => p::HELD,
        "QVS2404" => p::GROUP_SPLIT,
        "QVS2405" | "QVS2406" | "QVS2407" | "QVS2408" | "QVS2409" | "QVS2410" | "QVS2411"
        | "QVS2412" | "QVS2413" | "QVS2414" => p::NOT_ASSIGNABLE,
        "QVS2420" => p::NOT_A_DRIVE,
        "QVS2421" => p::NOT_VIRTUAL,
        "QVS2016" => qsi::host::partition::msgid::ID_TAKEN,
        "QVS2017" => qsi::host::partition::msgid::ID_OUT_OF_RANGE,
        "QVS2501" => e::NOT_RUNNING,
        "QVS2502" => e::DEDICATED,
        "QVS2503" => e::OUTSIDE_PROFILE,
        "QVS2504" => e::VIRTUALS,
        "QVS2505" => e::POOL_SHORT,
        _ => return None,
    })
}

/// A refusal's notes, led by QSI's identifier for the first when it
/// has one: the host's own follow (`docs/QSI.md` section 7).
pub fn led(notes: Vec<Note>) -> Vec<Note> {
    let Some(ours) = notes.first().and_then(|n| qsi_identifier(&n.msgid)) else {
        return notes;
    };
    let mut all = vec![Note {
        msgid: ours.into(),
        text: notes[0].text.clone(),
        help: notes[0].help.clone(),
    }];
    all.extend(notes);
    all
}

/// Why a device cannot be given, as QSI's open blocker: the host's own
/// reasons after QSI's five.
pub fn blocker(b: pio::Blocker) -> qsi::host::host::Blocker {
    use qsi::host::host::Blocker as Q;
    match b {
        pio::Blocker::HostUses => Q::HostUses,
        pio::Blocker::NotReleased => Q::NotReleased,
        pio::Blocker::GroupNotReleased => Q::GroupNotReleased,
        pio::Blocker::NoReset => Q::NoReset,
        pio::Blocker::NoRemapping => Q::NoRemapping,
        pio::Blocker::NoIommu => Q::Other(6),
        pio::Blocker::NoMessageInterrupts => Q::Other(7),
        pio::Blocker::Fenced => Q::Other(8),
        pio::Blocker::Gone => Q::Other(9),
        pio::Blocker::GroupChanged => Q::Other(10),
    }
}

/// A device as QSI has it.
pub fn device(d: &pio::Device) -> qsi::host::host::Device {
    let hex = |s: &str| u32::from_str_radix(s, 16).unwrap_or(0);
    qsi::host::host::Device {
        location: d.location.clone(),
        address: d.address.clone(),
        class: hex(&d.class),
        vendor: u16::try_from(hex(&d.vendor)).unwrap_or(0),
        device: u16::try_from(hex(&d.device)).unwrap_or(0),
        description: d.description.clone(),
        released: d.released,
        assignable: d.assignable(),
        blocked: d.blocked.as_ref().map(|(b, why)| qsi::host::host::Blocked {
            reason: blocker(*b),
            note: note(why),
        }),
        group: d.group.unwrap_or(0),
        with: d.with.clone(),
        holder: d.holder.as_ref().map(|h| qsi::host::host::DeviceHolder {
            partition: h.partition,
            name: h.name.clone(),
            profile: h.profile.clone(),
        }),
    }
}

/// One message of QS/VS's, as a QSI note.
pub fn note(i: &qvsmsg::Issued) -> q::Note {
    q::Note {
        msgid: i.id.clone(),
        text: i.text.clone(),
        help: i.help.clone(),
    }
}

/// A partition's state.
pub fn state(s: partd::State) -> q::PartitionState {
    match s {
        partd::State::NotActivated => q::PartitionState::NotActivated,
        partd::State::Starting => q::PartitionState::Starting,
        partd::State::Running => q::PartitionState::Running,
        partd::State::ShuttingDown => q::PartitionState::ShuttingDown,
        partd::State::Error => q::PartitionState::Error,
    }
}

/// A keylock position.
pub fn keylock(k: model::Keylock) -> QKeylock {
    match k {
        model::Keylock::Normal => QKeylock::Normal,
        model::Keylock::Manual => QKeylock::Manual,
        model::Keylock::Auto => QKeylock::Auto,
        model::Keylock::Secure => QKeylock::Secure,
    }
}

/// A console's keylock position.
pub fn keylock_in(k: QKeylock) -> Result<model::Keylock, Note> {
    Ok(match k {
        QKeylock::Normal => model::Keylock::Normal,
        QKeylock::Manual => model::Keylock::Manual,
        QKeylock::Auto => model::Keylock::Auto,
        QKeylock::Secure => model::Keylock::Secure,
        QKeylock::Other(_) => return Err(no_such("keylock position")),
    })
}

/// An IPL source.
pub fn source(s: model::IplSource) -> q::IplSource {
    match s {
        model::IplSource::A => q::IplSource::A,
        model::IplSource::B => q::IplSource::B,
        model::IplSource::Recorded => q::IplSource::Recorded,
        model::IplSource::D => q::IplSource::D,
    }
}

/// A console's IPL source.
pub fn source_in(s: q::IplSource) -> Result<model::IplSource, Note> {
    Ok(match s {
        q::IplSource::A => model::IplSource::A,
        q::IplSource::B => model::IplSource::B,
        q::IplSource::Recorded => model::IplSource::Recorded,
        q::IplSource::D => model::IplSource::D,
        q::IplSource::Other(_) => return Err(no_such("IPL source")),
    })
}

/// An IPL type.
pub fn ipl_type(t: model::IplType) -> q::IplType {
    match t {
        model::IplType::Unattended => q::IplType::Unattended,
        model::IplType::Attended => q::IplType::Attended,
    }
}

/// A console's IPL type.
pub fn ipl_type_in(t: q::IplType) -> Result<model::IplType, Note> {
    Ok(match t {
        q::IplType::Unattended => model::IplType::Unattended,
        q::IplType::Attended => model::IplType::Attended,
        q::IplType::Other(_) => return Err(no_such("IPL type")),
    })
}

fn console(c: model::ConsoleMode) -> q::ConsoleMode {
    match c {
        model::ConsoleMode::Serial => q::ConsoleMode::Serial,
        model::ConsoleMode::Local => q::ConsoleMode::Local,
        model::ConsoleMode::Partition => q::ConsoleMode::Partition,
    }
}

fn console_in(c: q::ConsoleMode) -> Result<model::ConsoleMode, Note> {
    Ok(match c {
        q::ConsoleMode::Serial => model::ConsoleMode::Serial,
        q::ConsoleMode::Local => model::ConsoleMode::Local,
        q::ConsoleMode::Partition => model::ConsoleMode::Partition,
        q::ConsoleMode::Other(_) => return Err(no_such("console mode")),
    })
}

/// Auto start settings.
pub fn auto_start(a: &model::AutoStart) -> q::AutoStart {
    q::AutoStart {
        enabled: a.enabled,
        order: a.order,
        delay_secs: a.delay_secs,
        wait: a.wait,
        wait_secs: a.wait_secs,
    }
}

/// A partition's definition.
pub fn definition(d: &model::Definition) -> q::Definition {
    q::Definition {
        id: d.id,
        name: d.name.clone(),
        environment: q::Environment::Standalone,
        serial: d.serial.clone(),
        keylock: keylock(d.keylock),
        default_profile: d.default_profile.clone(),
        auto_start: auto_start(&d.auto_start),
        created: d.created,
    }
}

/// A console's definition over the one kept: what QSI carries changes,
/// the rest (the serial number, when it was made, its revision) is
/// the host's.
pub fn definition_in(
    d: &q::Definition,
    kept: &model::Definition,
) -> Result<model::Definition, Note> {
    if let q::Environment::Other(_) = d.environment {
        return Err(no_such("environment"));
    }
    Ok(model::Definition {
        id: kept.id,
        name: d.name.clone(),
        environment: kept.environment,
        serial: d.serial.clone(),
        keylock: keylock_in(d.keylock)?,
        default_profile: d.default_profile.clone(),
        auto_start: model::AutoStart {
            enabled: d.auto_start.enabled,
            order: d.auto_start.order,
            delay_secs: d.auto_start.delay_secs,
            wait: d.auto_start.wait,
            wait_secs: d.auto_start.wait_secs,
        },
        created: kept.created,
        revision: kept.revision,
        // Changed by version 3's own request, never by a definition's.
        settings: kept.settings.clone(),
    })
}

/// A partition's settings as QSI has them.
pub fn settings(s: &model::PartitionSettings) -> qsi::host::partition::Settings {
    use qsi::host::partition::WithHost as W;
    qsi::host::partition::Settings {
        description: s.description.clone(),
        panel: s.panel,
        with_host: match s.with_host {
            model::WithHost::Delayed => W::Delayed,
            model::WithHost::Immediate => W::Immediate,
        },
        delay: s.delay_secs,
    }
}

/// A console's settings as the host keeps them.
pub fn settings_in(s: &qsi::host::partition::Settings) -> Result<model::PartitionSettings, Note> {
    use qsi::host::partition::WithHost as W;
    Ok(model::PartitionSettings {
        description: s.description.clone(),
        panel: s.panel,
        with_host: match s.with_host {
            W::Delayed => model::WithHost::Delayed,
            W::Immediate => model::WithHost::Immediate,
            W::Other(_) => return Err(no_such("shut down with the host")),
        },
        delay_secs: s.delay,
    })
}

fn units(u: Option<Units>) -> Option<u32> {
    u.map(|u| u.0)
}

/// A profile.
pub fn profile(p: &model::Profile) -> q::Profile {
    let c = &p.processors;
    q::Profile {
        name: p.name.clone(),
        processors: q::Processors {
            mode: match c.mode {
                model::ProcessorMode::Dedicated => q::ProcessorMode::Dedicated,
                model::ProcessorMode::Shared => q::ProcessorMode::Shared,
            },
            minimum: c.minimum,
            desired: c.desired,
            maximum: c.maximum,
            minimum_units: units(c.minimum_units),
            desired_units: units(c.desired_units),
            maximum_units: units(c.maximum_units),
            sharing: c.sharing.map(|s| match s {
                model::Sharing::Capped => q::Sharing::Capped,
                model::Sharing::Uncapped => q::Sharing::Uncapped,
            }),
            weight: c.weight,
            pool: c.pool.clone(),
        },
        memory: q::Memory {
            minimum_mib: p.memory.minimum_mib,
            desired_mib: p.memory.desired_mib,
            maximum_mib: p.memory.maximum_mib,
        },
        boot: q::Boot {
            source: source(p.boot.source),
            ipl_type: ipl_type(p.boot.ipl_type),
            console: console(p.boot.console),
        },
        scsi: p
            .scsi
            .iter()
            .map(|a| q::ScsiAdapter {
                slot: a.slot,
                luns: a
                    .luns
                    .iter()
                    .map(|l| q::Lun {
                        lun: l.lun,
                        backing: l.backing.clone(),
                        serial: l.serial.clone(),
                    })
                    .collect(),
            })
            .collect(),
        ethernet: p
            .ethernet
            .iter()
            .map(|e| q::EthernetAdapter {
                slot: e.slot,
                switch: e.switch.clone(),
                vlan: e.vlan,
                tagged: e.tagged.clone(),
                mac: e.mac.clone(),
                required: e.required,
            })
            .collect(),
        optical: media(&p.optical),
        tape: media(&p.tape),
        physical_io: p
            .physical_io
            .iter()
            .map(|io| q::PhysicalIo {
                location: io.location.clone(),
                required: io.required,
            })
            .collect(),
    }
}

fn media(devices: &[model::MediaDevice]) -> Vec<q::MediaDevice> {
    devices
        .iter()
        .map(|m| q::MediaDevice {
            slot: m.slot,
            media: m.media.clone(),
        })
        .collect()
}

fn media_in(devices: &[q::MediaDevice]) -> Vec<model::MediaDevice> {
    devices
        .iter()
        .map(|m| model::MediaDevice {
            slot: m.slot,
            media: m.media.clone(),
        })
        .collect()
}

/// A console's profile, as QS/VS keeps it; the revision is the put's.
pub fn profile_in(p: &q::Profile) -> Result<model::Profile, Note> {
    let c = &p.processors;
    let mode = match c.mode {
        q::ProcessorMode::Dedicated => model::ProcessorMode::Dedicated,
        q::ProcessorMode::Shared => model::ProcessorMode::Shared,
        q::ProcessorMode::Other(_) => return Err(no_such("processor mode")),
    };
    let sharing = match c.sharing {
        None => None,
        Some(q::Sharing::Capped) => Some(model::Sharing::Capped),
        Some(q::Sharing::Uncapped) => Some(model::Sharing::Uncapped),
        Some(q::Sharing::Other(_)) => return Err(no_such("sharing mode")),
    };
    Ok(model::Profile {
        name: p.name.clone(),
        processors: model::Processors {
            mode,
            minimum: c.minimum,
            desired: c.desired,
            maximum: c.maximum,
            minimum_units: c.minimum_units.map(Units),
            desired_units: c.desired_units.map(Units),
            maximum_units: c.maximum_units.map(Units),
            sharing,
            weight: c.weight,
            pool: c.pool.clone(),
        },
        memory: model::Memory {
            minimum_mib: p.memory.minimum_mib,
            desired_mib: p.memory.desired_mib,
            maximum_mib: p.memory.maximum_mib,
        },
        boot: model::Boot {
            source: source_in(p.boot.source)?,
            ipl_type: ipl_type_in(p.boot.ipl_type)?,
            console: console_in(p.boot.console)?,
        },
        scsi: p
            .scsi
            .iter()
            .map(|a| model::ScsiAdapter {
                slot: a.slot,
                luns: a
                    .luns
                    .iter()
                    .map(|l| model::Lun {
                        lun: l.lun,
                        backing: l.backing.clone(),
                        serial: l.serial.clone(),
                    })
                    .collect(),
            })
            .collect(),
        ethernet: p
            .ethernet
            .iter()
            .map(|e| model::EthernetAdapter {
                slot: e.slot,
                switch: e.switch.clone(),
                vlan: e.vlan,
                tagged: e.tagged.clone(),
                mac: e.mac.clone(),
                required: e.required,
            })
            .collect(),
        optical: media_in(&p.optical),
        tape: media_in(&p.tape),
        physical_io: p
            .physical_io
            .iter()
            .map(|io| model::PhysicalIo {
                location: io.location.clone(),
                required: io.required,
            })
            .collect(),
        revision: 0,
    })
}

/// What an activated partition holds.
pub fn current(c: &resource::Current) -> q::Current {
    match &c.processors {
        resource::Processors::Dedicated { cores } => q::Current {
            mode: q::ProcessorMode::Dedicated,
            cores: cores.clone(),
            pool: String::new(),
            units: Units::cores(u32::try_from(cores.len()).unwrap_or(u32::MAX)).0,
            virtuals: u32::try_from(cores.len()).unwrap_or(u32::MAX),
            capped: true,
            weight: 0,
            memory_mib: c.memory_mib,
        },
        resource::Processors::Shared {
            pool,
            units,
            virtuals,
            capped,
            weight,
        } => q::Current {
            mode: q::ProcessorMode::Shared,
            cores: Vec::new(),
            pool: pool.clone(),
            units: units.0,
            virtuals: *virtuals,
            capped: *capped,
            weight: *weight,
            memory_mib: c.memory_mib,
        },
    }
}

/// A partition's service channel.
pub fn channel(s: &partd::proto::Service) -> q::Channel {
    q::Channel {
        up: s.up,
        none: s.none,
        responding: s.responding,
        kept: s.kept,
        system: s.system.clone(),
        release: s.release.clone(),
        fix_level: s.fix_level.clone(),
        addresses: s.addresses.clone(),
        version: s.version,
        caps: s.caps,
    }
}

/// A partition's status.
pub fn status(s: &partd::Status) -> qsi::host::partitions::Status {
    qsi::host::partitions::Status {
        id: s.id,
        name: s.name.clone(),
        state: state(s.state),
        code: s.code.clone(),
        text: s.text.clone(),
        current: s.current.as_ref().map(current),
        profile: s.profile.clone(),
        last_end: s.last_end.clone(),
        message: s.message.as_ref().map(note),
        ethernet: s
            .ethernet
            .iter()
            .map(|e| qsi::host::partitions::Link {
                slot: e.slot,
                switch: e.switch.clone(),
                mac: e.mac.clone(),
                link: e.link,
            })
            .collect(),
        channel: channel(&s.service),
    }
}

/// A partition's posted code, `seq` its place in the history.
pub fn posted(p: &partd::Posted, seq: u64) -> qsi::host::refcodes::Posted {
    use qsi::host::refcodes::Source;
    qsi::host::refcodes::Posted {
        refcode: Refcode {
            seq,
            code: p.code.clone(),
            words: p.words.clone(),
            text: p.text.clone(),
            time_ms: p.time.saturating_mul(1000),
        },
        source: match p.source {
            partd::proto::Source::Host => Source::Host,
            partd::proto::Source::System => Source::System,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_reads_back_the_same() {
        for p in [model::sample::dedicated(), model::sample::shared()] {
            let back = profile_in(&profile(&p)).unwrap();
            assert_eq!(back, model::Profile { revision: 0, ..p });
        }
    }

    #[test]
    fn a_value_this_host_has_not_is_refused() {
        let mut p = profile(&model::sample::dedicated());
        p.boot.console = q::ConsoleMode::Other(9);
        assert_eq!(profile_in(&p).unwrap_err().msgid, NO_SUCH_VALUE);
        assert!(keylock_in(QKeylock::Other(7)).is_err());
    }

    #[test]
    fn a_definition_keeps_what_qsi_does_not_carry() {
        let kept = model::sample::definition();
        let mut d = definition(&kept);
        d.name = "OTHER".into();
        d.created = 1;
        let back = definition_in(&d, &kept).unwrap();
        assert_eq!(back.name, "OTHER");
        assert_eq!((back.created, back.revision), (kept.created, kept.revision));
    }
}
