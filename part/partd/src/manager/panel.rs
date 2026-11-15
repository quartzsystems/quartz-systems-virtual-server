// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's control panel functions (`docs/PANEL.md`). The host
//! carries out every one but 21, which the system does over its service
//! channel. A function that changes or interrupts the system needs the
//! keylock in manual, the host's keylock of this activation.
//!
//! | Function | What | Keylock |
//! |---|---|---|
//! | 01 | display the IPL source, IPL type and keylock | any |
//! | 02 | select them for the next activation | any |
//! | 03 | restart the IPL now | manual |
//! | 08 | power off now | manual |
//! | 11 to 19 | display a word of the current reference code | any |
//! | 20 | display the type, serial number and the host's release | any |
//! | 21 | dedicated service tools on the console | manual |
//! | 22 | dump, then restart the IPL | manual |
//! | 34 | take the dump again from the memory held | manual |

use model::{IplSource, IplType, Keylock};
use svcchan::Ask;

use super::service::Asked;
use super::{Answer, Manager, System};
use crate::proto::{PanelArgs, Refused};

/// The functions a partition has.
pub const FUNCTIONS: [u8; 18] = [
    1, 2, 3, 8, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 34, 0,
];

fn source_word(s: IplSource) -> &'static str {
    match s {
        IplSource::A => "A",
        IplSource::B => "B",
        IplSource::D => "D",
        IplSource::Recorded => "the side the partition asked for",
    }
}

fn type_word(t: IplType) -> &'static str {
    match t {
        IplType::Unattended => "unattended",
        IplType::Attended => "attended",
    }
}

impl<S: System> Manager<S> {
    /// Carry out function `function` of `id` with `args`: what it
    /// displays now, or the system's answer later.
    pub fn begin_panel(&mut self, id: u32, function: u8, args: PanelArgs) -> Answer {
        match self.panel_now(id, function, args) {
            Err(refused) => Answer::Now(Err(refused)),
            Ok(Some(text)) => Answer::Now(Ok(text)),
            Ok(None) => {
                let asked = self.ask(id, Ask::Panel(function), Asked::Panel);
                match asked {
                    Ok(rx) => Answer::Later(rx),
                    Err(e) => Answer::Now(Err(e)),
                }
            }
        }
    }

    /// A function carried out by the host: its text; `None` for one the
    /// system is to carry out.
    #[allow(clippy::too_many_lines)] // one arm a function
    fn panel_now(
        &mut self,
        id: u32,
        function: u8,
        args: PanelArgs,
    ) -> Result<Option<String>, Refused> {
        let one = |m: qvsmsg::Issued| vec![m];
        if function == 0 || !FUNCTIONS.contains(&function) {
            return Err(one(
                qvsmsg::PANEL_FUNCTION_INVALID.with(&[&format!("{function:02}")])
            ));
        }
        let p = self.partition(id)?;
        let name = p.definition.name.clone();
        let profile = p
            .run
            .as_ref()
            .map_or(&p.definition.default_profile, |r| &r.profile);
        let boot = p.profiles.get(profile).map(|profile| profile.boot);
        // This activation's, or the next's.
        let keylock = match &p.run {
            Some(r) => r.keylock.unwrap_or(p.definition.keylock),
            None => p.next.keylock.unwrap_or(p.definition.keylock),
        };
        let active = p.run.is_some();
        let kept = p.run.as_ref().is_some_and(|r| r.kept);
        let run_type = p.run.as_ref().and_then(|r| r.ipl_type);
        let next = p.next;
        let current = p.current.clone().unwrap_or_default();
        let release = p.service.facts.release.clone();
        let environment = p.definition.environment.word();
        let serial = p.definition.serial.clone();
        let can_panel = p.service.can(qsipart::caps::PANEL);
        let state = p.state;
        let manual = keylock == Keylock::Manual;
        let needs_manual = matches!(function, 3 | 8 | 21 | 22 | 34);
        let needs_active = matches!(function, 3 | 8 | 21 | 22 | 34);
        if needs_active && !active {
            return Err(one(
                qvsmsg::PARTITION_NOT_ACTIVE.with(&[&name, state.word()])
            ));
        }
        if needs_manual && !manual {
            return Err(one(qvsmsg::PANEL_NOT_MANUAL.with(&[
                &format!("{function:02}"),
                &name,
                keylock.word(),
            ])));
        }
        let done = |m: &mut Self| {
            let code = &refcode::PANEL_DONE;
            m.code(
                id,
                code.code,
                &code.text_with(&format!("{name}, {function:02}")),
                true,
            );
        };
        match function {
            1 => {
                let (source, ipl_type) = match (active, boot) {
                    (true, Some(b)) => (b.source, b.ipl_type),
                    (false, Some(b)) => (
                        next.source.unwrap_or(b.source),
                        next.ipl_type.unwrap_or(b.ipl_type),
                    ),
                    (_, None) => (IplSource::Recorded, IplType::Unattended),
                };
                let ipl_type = run_type.unwrap_or(ipl_type);
                Ok(Some(format!(
                    "01  IPL source {}, IPL type {}, keylock {}",
                    source_word(source),
                    type_word(ipl_type),
                    keylock.word()
                )))
            }
            2 => {
                if args.source == Some(IplSource::Recorded) {
                    return Err(one(
                        qvsmsg::PANEL_ARGUMENT_INVALID.with(&["02", "the IPL source"])
                    ));
                }
                let p = self.partitions.get_mut(&id).ok_or_else(Vec::new)?;
                if let Some(s) = args.source {
                    p.next.source = Some(s);
                }
                if let Some(t) = args.ipl_type {
                    p.next.ipl_type = Some(t);
                }
                if let Some(k) = args.keylock {
                    p.next.keylock = Some(k);
                }
                let next = p.next;
                let boot = boot.unwrap_or_default();
                done(self);
                Ok(Some(format!(
                    "02  next activation: IPL source {}, IPL type {}, keylock {}",
                    source_word(next.source.unwrap_or(boot.source)),
                    type_word(next.ipl_type.unwrap_or(boot.ipl_type)),
                    next.keylock.unwrap_or(keylock).word()
                )))
            }
            3 => {
                self.end_one(id, true)?;
                done(self);
                Ok(Some(format!("03  {name}: its IPL is restarted")))
            }
            8 => {
                self.end_one(id, false)?;
                done(self);
                Ok(Some(format!("08  {name}: powered off")))
            }
            11..=19 => {
                let word = usize::from(function - 11);
                let value = if word == 0 {
                    current.code
                } else {
                    current.words.get(word - 1).cloned().unwrap_or_default()
                };
                let value = if value.is_empty() {
                    "00000000".into()
                } else {
                    value
                };
                Ok(Some(format!("{function}  {value}")))
            }
            20 => {
                let system = if release.is_empty() {
                    String::new()
                } else {
                    format!(", system {release}")
                };
                Ok(Some(format!(
                    "20  type {environment} partition, serial number {serial}, host QS/VS {}{system}",
                    release::RELEASE
                )))
            }
            21 => {
                if !can_panel {
                    return Err(one(qvsmsg::NO_CHANNEL.with(&[&name, "function 21"])));
                }
                Ok(None)
            }
            22 => {
                let dump = self.begin_dump(id, "function22", true)?;
                done(self);
                Ok(Some(format!(
                    "22  {name}: dump {dump} begun; its IPL restarts when it is written"
                )))
            }
            34 => {
                if !kept {
                    return Err(one(qvsmsg::NOT_HELD.with(&[&name])));
                }
                let dump = self.begin_dump(id, "function34", false)?;
                done(self);
                Ok(Some(format!("34  {name}: dump {dump} begun")))
            }
            _ => Err(one(
                qvsmsg::PANEL_FUNCTION_INVALID.with(&[&format!("{function:02}")])
            )),
        }
    }
}
