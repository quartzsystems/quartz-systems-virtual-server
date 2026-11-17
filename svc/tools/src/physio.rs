// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 10, `Work with I/O for partitions` (`docs/PHYSICALIO.md`):
//! every PCI device and physical tape drive of the host, what the host
//! uses each for, whether it is released and who holds it; a device
//! released with the rest of its isolation group, its location code
//! typed in full; one taken back when no profile names it; why one
//! cannot be released. The partition manager does the work.

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use partd::Calls as _;
use partd::proto::TapeDrive;

use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

const TITLE: &str = "Work with I/O for Partitions";
/// Rows of the list on one page; the foot is under them.
const ROWS: usize = 14;

fn partd() -> partd::Client {
    partd::Client::new(partd::proto::socket(release::RUN_DIR))
}

/// The answer of the partition manager as a panel takes it.
fn asked<T>(answer: Result<Result<T, partd::proto::Refused>, ipc::WireError>) -> Result<T, String> {
    match answer {
        Err(e) => Err(format!("The partition manager does not answer: {e}.")),
        Ok(Err(refused)) => Err(refused.first().map_or_else(
            || "The request was refused.".to_string(),
            ToString::to_string,
        )),
        Ok(Ok(v)) => Ok(v),
    }
}

/// One row of the list: a device or a drive.
#[derive(Clone, Debug)]
enum Row {
    Device(Box<pio::Device>),
    Drive(TapeDrive),
}

impl Row {
    fn location(&self) -> &str {
        match self {
            Row::Device(d) => &d.location,
            Row::Drive(d) => &d.location,
        }
    }

    fn description(&self) -> String {
        match self {
            Row::Device(d) => d.description.clone(),
            Row::Drive(d) => format!("tape drive {} {}", d.vendor, d.product),
        }
    }

    /// What it is, in a list's column.
    fn kind(&self) -> String {
        match self {
            Row::Device(d) => pio::device::kind(&d.class).to_string(),
            Row::Drive(d) => format!("tape drive {}", d.product),
        }
    }

    /// What the host uses it for, or where it is; in full, or as a
    /// list's column has room for.
    fn state(&self, full: bool) -> String {
        match self {
            Row::Device(d) => {
                if let Some(u) = d.uses.first() {
                    let words = if full { u.words() } else { u.short() };
                    return format!("host: {words}");
                }
                if let Some(h) = &d.holder {
                    return format!("held by {}", h.name);
                }
                if d.fenced {
                    return "fenced: did not reset".into();
                }
                if !d.present {
                    return "released, not found".into();
                }
                if d.released {
                    return "released".into();
                }
                if d.releasable.is_empty() {
                    "not released".into()
                } else {
                    "cannot be released".into()
                }
            }
            Row::Drive(d) => {
                if d.holder != 0 {
                    format!("held by {}", d.name)
                } else if !d.present {
                    "released, not found".into()
                } else if d.released {
                    "released".into()
                } else {
                    "not released".into()
                }
            }
        }
    }
}

/// Every device and drive, by location code; none when the partition
/// manager does not answer.
fn rows() -> Option<Vec<Row>> {
    let mut c = partd();
    let devices = c.devices().ok()?;
    let drives = c.tape_drives().ok()?;
    let mut rows: Vec<Row> = devices
        .into_iter()
        .map(|d| Row::Device(Box::new(d)))
        .chain(drives.into_iter().map(Row::Drive))
        .collect();
    rows.sort_by(|a, b| a.location().cmp(b.location()));
    Some(rows)
}

/// `Work with I/O for partitions`, `top` the first row shown.
#[derive(Default)]
pub struct Io {
    top: usize,
}

impl Panel<Env, Outcome> for Io {
    fn name(&self) -> &'static str {
        "tools-io"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Some(list) = rows() else {
            return kit::page(
                TITLE,
                &env.system(),
                &kit::lines(&["The partition manager does not answer."]),
                "F3=Exit   F12=Cancel",
            );
        };
        let all: Vec<String> = list
            .iter()
            .map(|r| {
                format!(
                    "{} {} {}",
                    kit::fit(r.location(), 25),
                    kit::fit(&r.kind(), 24),
                    kit::fit(&r.state(false), 25)
                )
            })
            .collect();
        let (lines, foot) = kit::window(&all, self.top, ROWS);
        let mut s = ui::list(
            TITLE,
            &env.system(),
            "2=Release   4=Take back   5=Display",
            "Opt Location                  Description              Use",
            &lines,
            "F1=Help   F3=Exit   F5=Refresh   F7=Backward   F8=Forward   F12=Cancel",
        );
        if lines.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(The host has no PCI device.)");
        } else {
            kit::put(&mut s, ui::BODY_ROW + 3 + ROWS, &format!("{foot:>76}"));
        }
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let Some(list) = rows() else {
            return Flow::Error("The partition manager does not answer.".into());
        };
        let chosen: Vec<(&Row, u32)> = list
            .iter()
            .skip(self.top.min(list.len().saturating_sub(1)))
            .zip(fields)
            .filter_map(|(r, f)| kit::number(Some(f)).map(|n| (r, n)))
            .collect();
        let [(row, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one device.".into())
            };
        };
        match option {
            2 => Flow::Push(Box::new(Release {
                row: (*row).clone(),
            })),
            4 => Flow::Push(Box::new(TakeBack {
                location: row.location().to_string(),
            })),
            5 => Flow::Push(Box::new(Display {
                row: (*row).clone(),
            })),
            _ => Flow::Error("Option not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            7 | 8 => {
                let len = rows().map_or(0, |all| all.len());
                self.top = kit::scroll(self.top, key, len, ROWS);
                Flow::Stay
            }
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "Every PCI device and physical tape drive of the host, by its",
                "location code: what the host uses it for, whether it is",
                "released for partitions, and which partition holds it.",
                "",
                "2 releases a device for partitions: a management console can",
                "then name it in a profile. The rest of its isolation group goes",
                "with it, and its location code is typed to confirm. 4 takes a",
                "device back once no profile names it and no partition holds it.",
                "5 shows a device whole, and why it cannot be released. F7 and",
                "F8 page through the list.",
                "",
                "The host keeps its load source's and its pools' controllers,",
                "the management interface, every uplink and the display.",
            ],
        }
    }
}

/// `text` in lines of `width` at most, broken between words.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The lines that show a device or drive whole.
fn whole(row: &Row) -> Vec<String> {
    match row {
        Row::Device(d) => {
            let mut lines = vec![
                format!("Location code . . . . :  {}", d.location),
                format!("Description . . . . . :  {}", d.description),
                format!("PCI address . . . . . :  {}", d.address),
                format!(
                    "Isolation group . . . :  {}",
                    d.group.map_or_else(|| "none".into(), |g| g.to_string())
                ),
                format!(
                    "Goes with . . . . . . :  {}",
                    if d.with.is_empty() {
                        "nothing else".into()
                    } else {
                        d.with.join(", ")
                    }
                ),
                format!(
                    "Reset . . . . . . . . :  {}",
                    if d.reset.is_empty() {
                        "cannot be reset alone".into()
                    } else {
                        d.reset.join(", ")
                    }
                ),
                format!("Interrupts  . . . . . :  {}", d.interrupts.join(", ")),
                format!("Use . . . . . . . . . :  {}", row.state(true)),
                String::new(),
            ];
            if d.releasable.is_empty() {
                lines.push("It can be released for partitions.".into());
            } else {
                lines.push("It cannot be released:".into());
                for why in &d.releasable {
                    lines.extend(wrap(&why.text, 74).into_iter().map(|l| format!("  {l}")));
                }
            }
            lines
        }
        Row::Drive(d) => vec![
            format!("Location code . . . . :  {}", d.location),
            format!("Description . . . . . :  {}", row.description()),
            format!("Serial number . . . . :  {}", d.serial),
            format!(
                "Cartridge . . . . . . :  {}",
                if d.loaded { "loaded" } else { "none" }
            ),
            format!("Use . . . . . . . . . :  {}", row.state(true)),
            String::new(),
            "A partition's tape device names it as @ and its location code.".into(),
        ],
    }
}

/// Option 5: one device or drive.
struct Display {
    row: Row,
}

impl Panel<Env, Outcome> for Display {
    fn name(&self) -> &'static str {
        "tools-io-display"
    }

    fn render(&self, env: &mut Env) -> Screen {
        kit::page(
            "Display I/O for Partitions",
            &env.system(),
            &whole(&self.row),
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
    }

    fn help(&self) -> Help {
        Help {
            title: "Display I/O for Partitions",
            lines: &[
                "A device as the host has it. Its isolation group is the devices",
                "its IOMMU cannot tell apart: they go to a partition together.",
                "",
                "A device can be released only when the IOMMU and interrupt",
                "remapping are on, no device of its group is the host's, each",
                "can be reset alone and signals message interrupts. No setting",
                "changes these rules.",
            ],
        }
    }
}

/// Option 2: a release, its location code typed in full.
struct Release {
    row: Row,
}

impl Panel<Env, Outcome> for Release {
    fn name(&self) -> &'static str {
        "tools-io-release"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Confirm Release for Partitions",
            &env.system(),
            &[FormLine {
                label: "Type its location code",
                len: 32,
                value: String::new(),
                hidden: false,
            }],
            "F1=Help   F12=Cancel",
        );
        let mut lines = vec![
            format!(
                "{} is given to the pass-through driver",
                self.row.location()
            ),
            "now: the host's drivers let go of it, and a partition whose".into(),
            "profile names it is given it at its activation.".into(),
        ];
        if let Row::Device(d) = &self.row {
            if d.with.is_empty() {
                lines.push("Nothing else goes with it.".into());
            } else {
                lines.push("These devices of its isolation group go with it:".into());
                lines.extend(d.with.iter().map(|w| format!("  {w}")));
            }
        }
        for (row, line) in (ui::BODY_ROW + 2..).zip(&lines) {
            kit::put(&mut s, row, line);
        }
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let typed = fields.first().map_or("", |f| f.trim()).to_string();
        let location = self.row.location().to_string();
        match asked(partd().release(location, typed)) {
            Ok(_) => Flow::Reset(Box::new(Io::default())),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm Release for Partitions",
            lines: &[
                "Type the device's location code in full, as the panel shows it,",
                "and press Enter. The device and every other device of its",
                "isolation group are released together. F12 goes back with",
                "nothing released.",
            ],
        }
    }
}

/// Option 4: a take-back.
struct TakeBack {
    location: String,
}

impl Panel<Env, Outcome> for TakeBack {
    fn name(&self) -> &'static str {
        "tools-io-take-back"
    }

    fn render(&self, env: &mut Env) -> Screen {
        kit::page(
            "Confirm Take Back",
            &env.system(),
            &[
                format!("{} and the rest of its isolation group are", self.location),
                "given back to the host's drivers.".into(),
                String::new(),
                "Press Enter to take it back, or F12 to cancel.".into(),
            ],
            "F1=Help   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        match asked(partd().take_back(self.location.clone())) {
            Ok(_) => Flow::Reset(Box::new(Io::default())),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm Take Back",
            lines: &[
                "A device is taken back only while no partition holds it and no",
                "profile names it. Take it out of every profile first.",
            ],
        }
    }
}
