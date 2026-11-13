// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A mirrored pair whose units each ran without the other. Each disk's
//! label says its own unit is in step and the other is not, and both
//! have been written to since: nothing on the disks says which the
//! operator wants. The service then assembles nothing and is not
//! ready. It says so, answers its socket, and waits to be told which
//! unit to keep; the other is then stale and is written again from it.

use std::fs;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use appliance::{Appliance, Event};
use loadsource::disks::{self, LoadSource};
use loadsource::set::{self, SetLabel};
use supervise::signal::Signals;

use super::{Msg, Shown, What, found_disks, text};
use crate::{APART, Host, MIRRORED, Status, Unit};

/// One unit that ran alone: its disk and the label that disk holds.
struct Alone {
    source: LoadSource,
    label: SetLabel,
    serial: String,
    location: String,
}

/// The pair, apart.
pub struct Apart {
    app: Appliance,
    /// Every load-source disk that is there.
    sources: Vec<LoadSource>,
    /// The units that ran alone, in unit order.
    units: Vec<Alone>,
}

impl Apart {
    /// The units of `sources` that ran apart; `None` when none did.
    pub fn find(app: &Appliance, host: Host, sources: &[LoadSource]) -> Option<Self> {
        let own = disks::own_labels(app.load_source, sources);
        let claims: Vec<_> = own
            .iter()
            .map(|(i, label)| (sources[*i].data_guid, label.clone()))
            .collect();
        let apart = set::apart(&claims);
        if apart.is_empty() {
            return None;
        }
        let inventory = (host.inventory)();
        let mut units: Vec<Alone> = apart
            .into_iter()
            .map(|at| {
                let (i, label) = &own[at];
                let source = sources[*i].clone();
                // The disk as the machine has it now; else as the
                // label last had it.
                let recorded = label
                    .members
                    .iter()
                    .find(|m| m.partition == source.data_guid);
                let (serial, location) = inventory
                    .iter()
                    .find(|d| d.name == source.disk)
                    .map(|d| (d.serial.clone(), d.location.clone()))
                    .or_else(|| recorded.map(|m| (m.serial.clone(), m.location.clone())))
                    .unwrap_or_default();
                Alone {
                    source,
                    label: label.clone(),
                    serial,
                    location,
                }
            })
            .collect();
        units.sort_by_key(|u| (number(u), u.source.disk.clone()));
        Some(Self {
            app: *app,
            sources: sources.to_vec(),
            units,
        })
    }

    /// The units, for status.
    fn units(&self) -> Vec<Unit> {
        self.units
            .iter()
            .map(|u| Unit {
                number: number(u),
                disk: u.source.disk.clone(),
                serial: u.serial.clone(),
                location: u.location.clone(),
                bytes: disks::disk_bytes(&u.source.disk),
                state: APART.into(),
                written: u.label.written,
            })
            .collect()
    }

    fn status(&self, failure: &str) -> Status {
        Status {
            protection: MIRRORED.into(),
            units: self.units(),
            choose: true,
            failure: failure.into(),
            ..Status::default()
        }
    }

    /// What the code says: each unit, where it is and when it last ran.
    fn detail(&self) -> String {
        let each: Vec<String> = self
            .units()
            .iter()
            .map(|u| {
                let place = if u.location.is_empty() {
                    String::new()
                } else {
                    format!(" at {}", u.location)
                };
                format!("unit {}{place}", u.number)
            })
            .collect();
        format!(
            "{} each ran without the other; choose the unit to keep",
            each.join(" and ")
        )
    }

    /// Keep the unit on `disk`: its label, with the others stale, goes
    /// to every disk, and no map of an earlier copy survives. What was
    /// done, for the code.
    fn keep(&self, disk: &str) -> Result<String, String> {
        let kept = self
            .units
            .iter()
            .find(|u| u.source.disk == disk)
            .ok_or_else(|| format!("disk {disk} is no unit that ran alone"))?;
        let present = |partition| self.sources.iter().any(|s| s.data_guid == partition);
        let label = kept.label.keeping(present, super::clock());
        let names = self.app.load_source;
        let (_, written) = disks::labels(names, &self.sources)
            .map_err(text)?
            .write(&label.encode())
            .map_err(text)?;
        if written != self.sources.len() {
            return Err(format!(
                "the label was written to {written} of {} disks",
                self.sources.len()
            ));
        }
        // A payload that is no map.
        disks::resume_maps(names, &self.sources)
            .map_err(text)?
            .write(&[0u8; 56])
            .map_err(text)?;
        let place = if kept.location.is_empty() {
            String::new()
        } else {
            format!(" at {}", kept.location)
        };
        Ok(format!(
            "unit {}{place}; the other is written again from it",
            number(kept)
        ))
    }
}

/// A unit's number: its member's place in its own label, from 1.
fn number(unit: &Alone) -> u32 {
    let place = unit
        .label
        .members
        .iter()
        .position(|m| m.partition == unit.source.data_guid)
        .unwrap_or(0);
    u32::try_from(place + 1).unwrap_or(u32::MAX)
}

/// Before the set is assembled: when its units ran apart, say so and
/// wait here until the operator has chosen the one to keep. Returns
/// with the disks' labels in agreement.
pub fn settle(
    app: &Appliance,
    host: Host,
    rx: &Receiver<Msg>,
    signals: &mut Signals,
    shown: &Shown,
) {
    let marker = crate::choose_marker(app);
    let sources = found_disks(app.load_source);
    let Some(apart) = Apart::find(app, host, &sources) else {
        // A marker of a server that ended before this one.
        let _ = fs::remove_file(&marker);
        return;
    };
    let post = |what, detail: &str| (app.post)(&Event::Storage { what, detail });
    post(What::PairSplit, &apart.detail());
    shown.set(apart.status(""));
    // PID 1 looks for this: it mounts nothing and shows the choice.
    if let Err(e) = fs::write(&marker, "") {
        eprintln!("{}: {e}", marker.display());
    }
    loop {
        if super::term(signals, Duration::from_millis(100)) {
            // Nothing is served and nothing was written.
            std::process::exit(0);
        }
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Keep(disk, reply) => match apart.keep(&disk) {
                    Ok(detail) => {
                        let _ = reply.send(Ok(()));
                        post(What::UnitKept, &detail);
                        let _ = fs::remove_file(&marker);
                        return;
                    }
                    Err(e) => {
                        eprintln!("the unit was not kept: {e}");
                        shown.set(apart.status(&e));
                        let _ = reply.send(Err(e));
                    }
                },
                Msg::Start(_, reply) | Msg::Replace(_, _, reply) => {
                    let _ = reply.send(Err("choose the unit to keep first".into()));
                }
            }
        }
    }
}
