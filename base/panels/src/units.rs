// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `Work with disk units`: the load-source units and their protection,
//! the machine's other disks, a second disk added to an unprotected
//! load source, a failed unit replaced and its rebuild watched. The
//! storage service (`base/stgd`) does the work; these panels ask it and
//! show what it says. The product gives the options it has beside
//! these, their help, and its main menu, where the watch goes back to.

use console::Screen;
use scrkit::panel::Flow;
use scrkit::ui;
use state::doc::inventory::Disk;
use stgd::{MIRRORED, Status, Unit};

use crate::kit;
use crate::panel::{Help, Named, Panel, PanelFlow};
use crate::tools::Confirm;

/// The panels' title.
pub const TITLE: &str = "Work with Disk Units";

/// What the panels ask of the appliance.
pub trait LoadSource: Named {
    /// The storage service's status; `None` when it does not answer.
    fn storage(&mut self) -> Option<Status>;
    /// The machine's disks, as its inventory has them.
    fn machine_disks(&mut self) -> Vec<Disk>;
    /// Add `disk` to an unprotected load source as its second unit.
    fn start_protection(&mut self, disk: &str) -> Result<(), String>;
    /// Put `disk` in the place of unit `number`.
    fn replace(&mut self, number: u32, disk: &str) -> Result<(), String>;
}

/// What the product adds: its own options after 1, 2 and 5, the help
/// of the whole panel, what follows the count of other disks, and its
/// main menu.
pub struct Product<E: ?Sized + 'static, O: 'static> {
    /// Its options, in order.
    pub options: &'static [(u8, &'static str)],
    /// What one of them opens; `None` for a number it has not.
    pub open: fn(u8) -> Option<PanelFlow<E, O>>,
    /// The panel's help.
    pub help: &'static [&'static str],
    /// After `Other disks: N`.
    pub others: &'static str,
    /// The main menu: where the watch of a rebuild goes back to.
    pub home: fn() -> Box<dyn Panel<E, O>>,
}

/// Why a disk is to be chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    /// As the second unit of an unprotected load source.
    Protect,
    /// In the place of this unit.
    Replace(u32),
}

fn unit_row(unit: &Unit, status: &Status) -> String {
    let mut state = unit.state.clone();
    if status.work_unit == unit.number && !status.work.is_empty() {
        state = format!("{state}, {} {}%", status.work, status.percent);
    }
    let size = if unit.bytes == 0 {
        String::new()
    } else {
        kit::size(unit.bytes)
    };
    format!(
        "{:>4}  {} {} {:>9}  {state}",
        unit.number,
        kit::fit(&unit.location, 23),
        kit::fit(&unit.serial, 18),
        size
    )
}

/// The disks of the machine that are no load-source unit.
fn other_disks(all: Vec<Disk>, status: &Status) -> Vec<Disk> {
    all.into_iter()
        .filter(|d| !status.units.iter().any(|u| u.disk == d.name))
        .filter(|d| !d.name.starts_with("sr") && !d.name.starts_with("ublkb"))
        .collect()
}

/// The lines of the display: protection, the units, the other disks.
fn lines(status: &Status, others: &[Disk], after: &str) -> Vec<String> {
    let protection = if status.protection == MIRRORED {
        if status.whole() {
            "Mirrored; both units are in step"
        } else {
            "Mirrored; NOT protected now: a unit is not in step"
        }
    } else if status.restart_needed {
        "Mirrored protection starts at the next IPL"
    } else {
        "None: the load source is one disk"
    };
    let mut out = vec![
        format!("{} {protection}", kit::dotted("Protection", 15)),
        String::new(),
        "Unit  Location                Serial                  Size  State".to_string(),
    ];
    out.extend(status.units.iter().map(|u| unit_row(u, status)));
    if !status.failure.is_empty() {
        out.push(kit::fit(&format!("Last request: {}", status.failure), 76));
    }
    out.push(String::new());
    // The selection lists them all; here, how many.
    out.push(format!("Other disks: {}{after}", others.len()));
    out
}

/// The unit a replacement would take the place of.
fn failed_unit(status: &Status) -> Option<u32> {
    status
        .units
        .iter()
        .find(|u| matches!(u.state.as_str(), "Missing" | "Failed" | "Suspended"))
        .map(|u| u.number)
}

/// `Work with disk units`.
pub struct DiskUnits<E: ?Sized + 'static, O: 'static> {
    product: &'static Product<E, O>,
}

impl<E: ?Sized + 'static, O: 'static> DiskUnits<E, O> {
    /// The panel, with what the product adds.
    pub fn new(product: &'static Product<E, O>) -> Self {
        Self { product }
    }
}

impl<E: LoadSource + ?Sized + 'static, O: 'static> Panel<E, O> for DiskUnits<E, O> {
    fn name(&self) -> &'static str {
        "tools-disks"
    }

    fn render(&self, env: &mut E) -> Screen {
        let Some(status) = env.storage() else {
            return kit::page(
                TITLE,
                &env.system(),
                &kit::lines(&["The storage service does not answer."]),
                "F3=Exit   F12=Cancel",
            );
        };
        let others = other_disks(env.machine_disks(), &status);
        let mut options = vec![
            (1, "Start mirrored protection"),
            (2, "Replace a failed unit"),
            (5, "Display the units again"),
        ];
        options.extend_from_slice(self.product.options);
        ui::menu_after(
            TITLE,
            &env.system(),
            &lines(&status, &others, self.product.others),
            &options,
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        let Some(status) = env.storage() else {
            return Flow::Error("The storage service does not answer.".into());
        };
        match kit::number(fields.first()) {
            Some(1) if status.protection == MIRRORED || status.restart_needed => {
                Flow::Error("The load source has mirrored protection already.".into())
            }
            Some(1) => Flow::Push(Box::new(Select {
                purpose: Purpose::Protect,
                product: self.product,
            })),
            Some(2) => match failed_unit(&status) {
                _ if !status.work.is_empty() => Flow::Push(Box::new(Watch {
                    product: self.product,
                })),
                Some(number) => Flow::Push(Box::new(Select {
                    purpose: Purpose::Replace(number),
                    product: self.product,
                })),
                None => Flow::Error("No unit is missing or failed.".into()),
            },
            Some(5) | None => Flow::Stay,
            Some(n) => (self.product.open)(u8::try_from(n).unwrap_or(0))
                .unwrap_or_else(|| Flow::Error("Selection not valid.".into())),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: self.product.help,
        }
    }
}

/// Choose one of the disks that are not configured.
struct Select<E: ?Sized + 'static, O: 'static> {
    purpose: Purpose,
    product: &'static Product<E, O>,
}

fn free_disks<E: LoadSource + ?Sized>(env: &mut E) -> Vec<Disk> {
    match env.storage() {
        Some(status) => other_disks(env.machine_disks(), &status),
        None => Vec::new(),
    }
}

impl<E: LoadSource + ?Sized + 'static, O: 'static> Panel<E, O> for Select<E, O> {
    fn name(&self) -> &'static str {
        "tools-disk-select"
    }

    fn render(&self, env: &mut E) -> Screen {
        let rows: Vec<String> = free_disks(env)
            .iter()
            .map(|d| {
                format!(
                    "{} {} {} {:>9}",
                    kit::fit(&d.location, 23),
                    kit::fit(&d.model, 19),
                    kit::fit(&d.serial, 18),
                    kit::size(d.bytes)
                )
            })
            .collect();
        let prompt = match self.purpose {
            Purpose::Protect => "Type 1 by the disk to mirror the load source with.".to_string(),
            Purpose::Replace(n) => format!("Type 1 by the disk to put in the place of unit {n}."),
        };
        let mut s = ui::list(
            TITLE,
            &env.system(),
            &prompt,
            "Opt Location                Model               Serial                  Size",
            &rows,
            "F1=Help   F3=Exit   F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(No disk is free)");
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        let disks = free_disks(env);
        let chosen: Vec<&Disk> = disks
            .iter()
            .zip(fields)
            .filter(|(_, option)| !option.trim().is_empty())
            .map(|(disk, _)| disk)
            .collect();
        let [disk] = chosen.as_slice() else {
            return Flow::Error("Type 1 by one disk.".into());
        };
        let purpose = self.purpose;
        let product = self.product;
        let name = disk.name.clone();
        let mut text = vec![
            "Everything on this disk will be erased:".to_string(),
            String::new(),
            format!(
                "  {}  {}  serial {}  {}",
                disk.location,
                disk.model,
                disk.serial,
                kit::size(disk.bytes)
            ),
            String::new(),
        ];
        text.push(match purpose {
            Purpose::Protect => {
                "It becomes unit 2 of the load source. Mirrored protection starts at".to_string()
            }
            Purpose::Replace(n) => {
                format!("It takes the place of unit {n} and is rebuilt from the other unit")
            }
        });
        text.push(match purpose {
            Purpose::Protect => "the next IPL, when the disk is written from unit 1.".to_string(),
            Purpose::Replace(_) => "while the host runs.".to_string(),
        });
        Flow::Push(Box::new(Confirm::new(
            "tools-disk-confirm",
            "Confirm Erase of Disk",
            text,
            Help {
                title: "Confirm Erase of Disk",
                lines: &[
                    "F10 erases the disk named and makes it a load-source unit:",
                    "it gets the partitions, both slots and the records the",
                    "other unit has, and then that unit's data.",
                    "",
                    "Nothing on the disk survives. F12 goes back.",
                ],
            },
            move |env: &mut E| {
                let asked = match purpose {
                    Purpose::Protect => env.start_protection(&name),
                    Purpose::Replace(n) => env.replace(n, &name),
                };
                match asked {
                    Ok(()) => Flow::Reset(Box::new(Watch { product })),
                    Err(e) => Flow::Unwind(2, Some((e, true))),
                }
            },
        )))
    }

    fn help(&self) -> Help {
        Help {
            title: "Select a disk",
            lines: &[
                "The disks of this machine that are no load-source unit, each by",
                "its location code, model, serial number and size.",
                "",
                "Type 1 by the one to use and press Enter. The disk must be at",
                "least as large as the load source's data needs; the storage",
                "service says so if it is not. The next screen asks you to",
                "confirm, and nothing is written before that.",
            ],
        }
    }
}

/// The storage service at work, watched.
struct Watch<E: ?Sized + 'static, O: 'static> {
    product: &'static Product<E, O>,
}

impl<E: LoadSource + ?Sized + 'static, O: 'static> Panel<E, O> for Watch<E, O> {
    fn name(&self) -> &'static str {
        "tools-disk-watch"
    }

    fn render(&self, env: &mut E) -> Screen {
        let Some(status) = env.storage() else {
            return kit::page(
                TITLE,
                &env.system(),
                &kit::lines(&["The storage service does not answer; it is asked again."]),
                "Enter=Continue",
            );
        };
        let mut text =
            vec!["Unit  Location                Serial                  Size  State".to_string()];
        text.extend(status.units.iter().map(|u| unit_row(u, &status)));
        text.push(String::new());
        let percent = match status.work.as_str() {
            "" if !status.failure.is_empty() => {
                text.push("The request was not carried out:".into());
                text.push(kit::fit(&status.failure, 76));
                None
            }
            "" if status.restart_needed => {
                text.push(
                    "The disk is prepared. Mirrored protection starts at the next IPL:".into(),
                );
                text.push("restart the host from the main menu when it suits.".into());
                None
            }
            "" if status.whole() => {
                text.push("Both units are in step. The load source is protected.".into());
                None
            }
            "" => None,
            "preparing" => {
                text.push(format!("Unit {} is being prepared.", status.work_unit));
                None
            }
            work => {
                text.push(format!("Unit {} is {work}.", status.work_unit));
                Some(u8::try_from(status.percent.min(100)).unwrap_or(100))
            }
        };
        kit::enterable(ui::text(
            TITLE,
            &env.system(),
            &text,
            percent,
            "Enter=Continue   F1=Help",
        ))
    }

    fn enter(&mut self, _fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        Flow::Reset((self.product.home)())
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        match key {
            3 | 12 => Flow::Reset((self.product.home)()),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Disk unit at work",
            lines: &[
                "The storage service is preparing a disk or writing a unit from",
                "the other. The display follows it; the bar is how much of the",
                "unit is in step.",
                "",
                "The host runs on meanwhile and nothing waits for this display:",
                "Enter goes back to the main menu and the work goes on. Option 2",
                "of the main menu shows where it stands.",
            ],
        }
    }

    fn live(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(number: u32, state: &str, disk: &str) -> Unit {
        Unit {
            number,
            disk: disk.into(),
            serial: format!("DEMOHD{}", number - 1),
            location: format!("P0000-B00-D1{}-F0", number - 1),
            bytes: if disk.is_empty() { 0 } else { 671_088_640 },
            state: state.into(),
            written: 0,
        }
    }

    #[test]
    fn the_units_are_shown_by_location_with_their_state_and_the_work() {
        let status = Status {
            protection: MIRRORED.into(),
            device: "/dev/ublkb0".into(),
            units: vec![unit(1, "Active", "vda"), unit(2, "Resuming", "vdb")],
            work: "rebuilding".into(),
            work_unit: 2,
            percent: 40,
            ..Status::default()
        };
        let shown = lines(&status, &[], " (elsewhere)");
        assert!(shown[0].contains("NOT protected now"));
        assert!(shown[3].starts_with("   1  P0000-B00-D10-F0"));
        assert!(shown[3].ends_with("671.0 MB  Active"));
        assert!(shown[4].ends_with("Resuming, rebuilding 40%"));
        assert_eq!(shown.last().unwrap(), "Other disks: 0 (elsewhere)");
        assert_eq!(failed_unit(&status), None);
    }

    #[test]
    fn a_missing_unit_is_the_one_a_replacement_takes_the_place_of() {
        let status = Status {
            protection: MIRRORED.into(),
            units: vec![unit(1, "Missing", ""), unit(2, "Active", "vdb")],
            failure: "unit 1: no".into(),
            ..Status::default()
        };
        assert_eq!(failed_unit(&status), Some(1));
        let shown = lines(&status, &[], "");
        // A unit that is gone still has its location and serial.
        assert!(shown[3].contains("P0000-B00-D10-F0") && shown[3].ends_with("Missing"));
        assert!(shown[5].starts_with("Last request: unit 1: no"));
        let plain = Status {
            units: vec![unit(1, "Unprotected", "vda")],
            ..Status::default()
        };
        assert!(lines(&plain, &[], "")[0].contains("None: the load source is one disk"));
    }
}
