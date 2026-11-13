// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager, from the harness. A request is encoded here
//! as the manager's own client encodes it, carried by the test hooks'
//! channel to the manager's socket on the host, and the answer comes
//! back the same way: the harness speaks the protocol `docs/PARTD.md`
//! documents, to the socket a release image has too.

use std::io;
use std::time::{Duration, Instant};

use model::profile::{
    Boot, ConsoleMode, IplSource, IplType, Lun, Memory, ProcessorMode, Processors, Profile,
    ScsiAdapter, Sharing,
};
use model::{Keylock, Units};
use partd::{Calls, Request, State, Status};
use qsvshook::proto::{command, word};
use qvsmsg::Issued;

use crate::qemu::channel::Channel;
use crate::qemu::session::Session;

/// The partition manager behind a channel.
pub struct Partd<'a> {
    channel: &'a mut Channel,
}

impl Calls for Partd<'_> {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        let line = format!("{} {}", command::PARTD, hex::encode(ipc::encode(req)));
        let lines = self
            .channel
            .ask_within(&line, Duration::from_secs(20))
            .map_err(io::Error::other)?;
        let answer = lines
            .first()
            .and_then(|l| l.strip_prefix(word::HEX))
            .ok_or_else(|| io::Error::other(format!("`{}` answered {lines:?}", req.name())))?;
        Ok(hex::decode(answer.trim()).map_err(io::Error::other)?)
    }
}

/// The partition manager of the host `guest` is.
pub fn partd(guest: &mut Session) -> Result<Partd<'_>, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    Ok(Partd {
        channel: guest.channel(deadline)?,
    })
}

/// A request's failure as one line: every message.
pub fn refused(errors: &[Issued]) -> String {
    let all: Vec<String> = errors.iter().map(ToString::to_string).collect();
    all.join("; ")
}

/// What went wrong with a call: the channel, or the messages.
pub fn said<T>(result: Result<Result<T, Vec<Issued>>, ipc::WireError>) -> Result<T, String> {
    result
        .map_err(|e| format!("the partition manager: {e}"))?
        .map_err(|errors| refused(&errors))
}

/// A profile for the suites, by what differs between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The profile's name.
    pub name: String,
    /// Dedicated processors, or virtual processors when `units` is set.
    pub processors: u32,
    /// Shared: the processing units, and whether capped.
    pub units: Option<(Units, Sharing)>,
    /// The uncapped weight.
    pub weight: Option<u32>,
    /// The least and the desired memory, in MiB.
    pub memory_mib: (u64, u64),
    /// The IPL source.
    pub side: IplSource,
    /// The keylock position of an activation with this.
    pub keylock: Option<Keylock>,
    /// The IPL type.
    pub ipl_type: IplType,
    /// The console mode.
    pub console: ConsoleMode,
    /// The units' backing stores, in unit order.
    pub backing: Vec<String>,
    /// The load source's serial number.
    pub serial: Option<String>,
}

impl Spec {
    /// Dedicated processors and that much memory, the IPL from the
    /// side the partition asked for, its console on serial.
    pub fn new(processors: u32, memory_mib: u64, backing: &str) -> Self {
        Self {
            name: "TEST".into(),
            processors,
            units: None,
            weight: None,
            memory_mib: (memory_mib, memory_mib),
            side: IplSource::Recorded,
            keylock: None,
            ipl_type: IplType::Unattended,
            console: ConsoleMode::Serial,
            backing: vec![backing.to_string()],
            serial: None,
        }
    }

    /// The same from side `side`.
    #[must_use]
    pub fn side(mut self, side: IplSource) -> Self {
        self.side = side;
        self
    }

    /// The same with shared processors: `units` over the processors as
    /// virtual ones.
    #[must_use]
    pub fn shared(mut self, units: u32, sharing: Sharing) -> Self {
        self.units = Some((Units(units), sharing));
        self
    }

    /// The profile.
    pub fn profile(&self) -> Profile {
        let (minimum_mib, desired_mib) = self.memory_mib;
        let units = self.units.map(|(units, _)| units);
        Profile {
            name: self.name.clone(),
            processors: Processors {
                mode: if units.is_some() {
                    ProcessorMode::Shared
                } else {
                    ProcessorMode::Dedicated
                },
                minimum: self.processors,
                desired: self.processors,
                maximum: self.processors,
                minimum_units: units,
                desired_units: units,
                maximum_units: units,
                sharing: self.units.map(|(_, sharing)| sharing),
                weight: self.weight,
                pool: None,
            },
            memory: Memory {
                minimum_mib,
                desired_mib,
                maximum_mib: desired_mib,
            },
            boot: Boot {
                source: self.side,
                ipl_type: self.ipl_type,
                console: self.console,
            },
            scsi: vec![ScsiAdapter {
                slot: 2,
                luns: (0u32..)
                    .zip(&self.backing)
                    .map(|(lun, backing)| Lun {
                        lun,
                        backing: backing.clone(),
                        serial: self.serial.clone().filter(|_| lun == 0),
                    })
                    .collect(),
            }],
            ethernet: Vec::new(),
            optical: Vec::new(),
            tape: Vec::new(),
            physical_io: Vec::new(),
        }
    }
}

/// The partition called `name`, created when it is not there; its ID.
pub fn partition(guest: &mut Session, name: &str) -> Result<u32, String> {
    let mut partd = partd(guest)?;
    let all = partd.list().map_err(|e| e.to_string())?;
    if let Some(found) = all.iter().find(|p| p.name == name) {
        return Ok(found.id);
    }
    said(partd.create(name.to_string())).map(|definition| definition.id)
}

/// Keep `spec` as a profile of the partition `id`.
pub fn put(guest: &mut Session, id: u32, spec: &Spec) -> Result<(), String> {
    said(partd(guest)?.put_profile(id, spec.profile()))
}

/// Activate the partition `id` with `spec`'s profile; what it was given.
pub fn activate(
    guest: &mut Session,
    id: u32,
    spec: &Spec,
) -> Result<resource::Current, Vec<Issued>> {
    let failed = |e: String| vec![qvsmsg::STATE_NOT_KEPT.with(&["The request", &e])];
    put(guest, id, spec).map_err(failed)?;
    partd(guest)
        .map_err(failed)?
        .activate(id, spec.name.clone(), None, spec.keylock)
        .map_err(|e| failed(e.to_string()))?
}

/// Flag the partition `id` to start with the host, at `order` after
/// `delay_secs` seconds.
pub fn auto_start(guest: &mut Session, id: u32, order: u32, delay_secs: u32) -> Result<(), String> {
    let mut definition = said(partd(guest)?.definition(id))?;
    definition.auto_start = model::AutoStart {
        enabled: true,
        order,
        delay_secs,
    };
    // A definition changes only while its partition is not activated:
    // the flag is what is kept, and the partition goes on.
    match partd(guest)?.change(definition.clone()) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(errors)) => Err(refused(&errors)),
        Err(e) => Err(e.to_string()),
    }
}

/// The partition `id`, as the manager's list has it.
pub fn status(guest: &mut Session, id: u32) -> Result<Status, String> {
    partd(guest)?
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("the partition manager lists no partition {id}"))
}

/// Wait until the partition `id` is in `state`.
pub fn until(
    guest: &mut Session,
    id: u32,
    state: State,
    deadline: Instant,
) -> Result<Status, String> {
    loop {
        let now = status(guest, id)?;
        if now.state == state {
            return Ok(now);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "partition {id} is {}, not {}, with `{} {}`{}",
                now.state.word(),
                state.word(),
                now.code,
                now.text,
                now.message.map(|m| format!("; {m}")).unwrap_or_default()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Wait until the partition `id` has no VMM; its status then.
pub fn ended(guest: &mut Session, id: u32, deadline: Instant) -> Result<Status, String> {
    loop {
        let now = status(guest, id)?;
        if !now.state.active() {
            return Ok(now);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "partition {id} did not end in time: it is {}",
                now.state.word()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spec_is_a_profile_the_model_takes() {
        let limits = model::Limits {
            memory_block_mib: 16,
        };
        let dedicated = Spec::new(2, 2048, "/dev/vdb").side(IplSource::B);
        let profile = dedicated.profile();
        assert_eq!(model::validate::profile(&profile, limits), []);
        assert_eq!(profile.boot.source, IplSource::B);
        assert_eq!(profile.processors.mode, ProcessorMode::Dedicated);
        assert_eq!(profile.scsi[0].luns[0].backing, "/dev/vdb");
        let mut shared = Spec::new(1, 1024, "/dev/vdb").shared(50, Sharing::Capped);
        shared.backing.push("/dev/vdc".into());
        shared.memory_mib = (512, 1024);
        let profile = shared.profile();
        assert_eq!(model::validate::profile(&profile, limits), []);
        assert_eq!(profile.processors.desired_units, Some(Units(50)));
        assert_eq!(profile.memory.minimum_mib, 512);
        assert_eq!(profile.scsi[0].luns.len(), 2);
        assert_eq!(
            refused(&[
                qvsmsg::NO_LOAD_SOURCE.issue(),
                qvsmsg::POOL_MISSING.with(&["X"])
            ]),
            "QVS1204 The profile has no load source.; QVS2105 Shared processor pool X does not exist."
        );
    }
}
