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

/// One function the host carries out: what `docs/PANEL.md`'s table
/// says of it, which a unit test holds the document to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Function {
    /// Its number.
    pub number: u8,
    /// What it does.
    pub text: &'static str,
    /// It needs the keylock in manual.
    pub manual: bool,
    /// It changes or interrupts the system: a console asks first.
    pub confirm: bool,
}

const fn f(number: u8, text: &'static str, manual: bool, confirm: bool) -> Function {
    Function {
        number,
        text,
        manual,
        confirm,
    }
}

/// The functions a partition has, by number.
pub const TABLE: [Function; 17] = [
    f(
        1,
        "Display the IPL source, the IPL type and the keylock",
        false,
        false,
    ),
    f(
        2,
        "Select the IPL source, IPL type and keylock for the next activation",
        false,
        false,
    ),
    f(3, "Restart the partition's IPL now", true, true),
    f(8, "Power the partition off now", true, true),
    f(11, "Display the current reference code", false, false),
    f(
        12,
        "Display word 2 of the current reference code",
        false,
        false,
    ),
    f(
        13,
        "Display word 3 of the current reference code",
        false,
        false,
    ),
    f(
        14,
        "Display word 4 of the current reference code",
        false,
        false,
    ),
    f(
        15,
        "Display word 5 of the current reference code",
        false,
        false,
    ),
    f(
        16,
        "Display word 6 of the current reference code",
        false,
        false,
    ),
    f(
        17,
        "Display word 7 of the current reference code",
        false,
        false,
    ),
    f(
        18,
        "Display word 8 of the current reference code",
        false,
        false,
    ),
    f(
        19,
        "Display word 9 of the current reference code",
        false,
        false,
    ),
    f(
        20,
        "Display the partition's type, its serial number and the host's release",
        false,
        false,
    ),
    f(
        21,
        "Dedicated service tools on the partition's console",
        true,
        false,
    ),
    f(
        22,
        "Dump the partition's memory, then restart its IPL",
        true,
        true,
    ),
    f(
        34,
        "Take the dump again from the memory still held",
        true,
        false,
    ),
];

/// Whether the host carries out `function`.
fn has(function: u8) -> bool {
    TABLE.iter().any(|f| f.number == function)
}

/// Whether `function` needs the keylock in manual.
fn needs_manual(function: u8) -> bool {
    TABLE.iter().any(|f| f.number == function && f.manual)
}

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
        if !has(function) {
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
        // What needs manual interrupts the system: it needs one running.
        if needs_manual(function) && !active {
            return Err(one(
                qvsmsg::PARTITION_NOT_ACTIVE.with(&[&name, state.word()])
            ));
        }
        if needs_manual(function) && !manual {
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

#[cfg(test)]
mod tests {
    use super::TABLE;

    /// `docs/PANEL.md`'s table: each row's functions and keylock.
    #[test]
    fn the_document_is_written_from_the_table() {
        let doc = include_str!("../../../../docs/PANEL.md");
        let mut rows = Vec::new();
        for line in doc.lines().filter(|l| l.starts_with("| ")) {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let (number, keylock) = (cells[1], cells[cells.len() - 2]);
            let numbers: Vec<u8> = match number.split_once(" to ") {
                Some((a, b)) => match (a.parse::<u8>(), b.parse::<u8>()) {
                    (Ok(a), Ok(b)) => (a..=b).collect(),
                    _ => continue,
                },
                None => match number.parse() {
                    Ok(n) => vec![n],
                    Err(_) => continue,
                },
            };
            for n in numbers {
                rows.push((n, keylock == "manual"));
            }
        }
        let table: Vec<(u8, bool)> = TABLE.iter().map(|f| (f.number, f.manual)).collect();
        assert_eq!(rows, table, "docs/PANEL.md's table and panel::TABLE");
        let mut sorted = table.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, table, "by number");
    }
}
