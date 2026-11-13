// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The one panel a console shows before the data file system exists:
//! the units of a mirrored load source each ran without the other, and
//! the operator says which to keep. The storage service has the units
//! and does what is chosen; nobody is signed on, since the service
//! tools users are on the data file system this is about.

use console::Screen;
use scrkit::panel::Flow;
use scrkit::ui;

use crate::kit;
use crate::panel::{Help, Named, Panel, PanelFlow};
use crate::tools::Confirm;

/// The panel's title.
pub const TITLE: &str = "Select Load-Source Unit to Keep";

/// One unit that ran alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApartUnit {
    /// Its number, from 1.
    pub number: u32,
    /// The disk's kernel name: what the storage service is told.
    pub disk: String,
    /// The disk's location code.
    pub location: String,
    /// The disk's serial number.
    pub serial: String,
    /// When the disk was last written, as displayed.
    pub written: String,
}

/// What the panel asks of the appliance.
pub trait Units: Named {
    /// The units that ran apart: `None` when the storage service does
    /// not answer, none when it waits for no choice any more.
    fn apart(&mut self) -> Option<Vec<ApartUnit>>;
    /// Keep the unit on `disk`; the others are written again from it.
    fn keep(&mut self, disk: &str) -> Result<(), String>;
}

const HEADINGS: &str = "Opt Unit  Location                Serial              Last written";
const MUST: &str = "A unit must be chosen before the host can go on.";

fn row(unit: &ApartUnit) -> String {
    format!(
        "{:>4}  {} {}  {}",
        unit.number,
        kit::fit(&unit.location, 23),
        kit::fit(&unit.serial, 18),
        unit.written
    )
}

/// `Select Load-Source Unit to Keep`. It ends with the number of the
/// unit kept, or 0 when the storage service asked no more.
pub struct KeepUnit;

impl<E: Units + ?Sized + 'static> Panel<E, u32> for KeepUnit {
    fn name(&self) -> &'static str {
        "keep-unit"
    }

    fn render(&self, env: &mut E) -> Screen {
        let Some(units) = env.apart() else {
            return kit::page(
                TITLE,
                &env.system(),
                &kit::lines(&["The storage service does not answer. Press Enter to ask again."]),
                "Enter=Continue   F1=Help",
            );
        };
        let rows: Vec<String> = units.iter().map(row).collect();
        let mut s = ui::list(
            TITLE,
            &env.system(),
            "Each unit ran without the other. Type 1 by the unit to keep.",
            HEADINGS,
            &rows,
            "Enter=Continue   F1=Help",
        );
        let below = ui::BODY_ROW + 4 + rows.len();
        for (i, line) in [
            "The host has started nothing and waits for this choice. The unit",
            "that is not kept is written again from the one that is: whatever",
            "was written to it while it ran alone is lost.",
        ]
        .iter()
        .enumerate()
        {
            kit::put(&mut s, below + i, line);
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, u32> {
        let Some(units) = env.apart() else {
            return Flow::Stay;
        };
        if units.is_empty() {
            return Flow::Exit(0);
        }
        let chosen: Vec<&ApartUnit> = units
            .iter()
            .zip(fields)
            .filter(|(_, option)| !option.trim().is_empty())
            .map(|(unit, _)| unit)
            .collect();
        let [unit] = chosen.as_slice() else {
            return Flow::Error("Type 1 by one unit.".into());
        };
        let (number, disk) = (unit.number, unit.disk.clone());
        let others: Vec<String> = units
            .iter()
            .filter(|u| u.disk != disk)
            .map(|u| format!("  Unit {}  {}  serial {}", u.number, u.location, u.serial))
            .collect();
        let mut text = vec![
            "This unit is kept, and the host goes on from it:".to_string(),
            String::new(),
            format!(
                "  Unit {}  {}  serial {}",
                unit.number, unit.location, unit.serial
            ),
            format!("  last written {}", unit.written),
            String::new(),
            "Everything written to the other while it ran alone is lost. It is".to_string(),
            "written again from the unit kept:".to_string(),
            String::new(),
        ];
        text.extend(others);
        Flow::Push(Box::new(Confirm::new(
            "keep-unit-confirm",
            "Confirm Unit to Keep",
            text,
            Help {
                title: "Confirm Unit to Keep",
                lines: &[
                    "F10 keeps the unit named first. The host then mounts its",
                    "data from that unit and starts; the other unit is written",
                    "again from it while the host runs.",
                    "",
                    "What the other unit alone held cannot be had back",
                    "afterwards. F12 goes back to the units.",
                ],
            },
            move |env: &mut E| match env.keep(&disk) {
                Ok(()) => Flow::Exit(number),
                Err(e) => Flow::Unwind(1, Some((e, true))),
            },
        )))
    }

    fn function(&mut self, _key: u8, _fields: &[String], _env: &mut E) -> PanelFlow<E, u32> {
        Flow::Error(MUST.into())
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "The load source is a mirrored pair of disks. Each of the two",
                "was started without the other and went on alone, so each now",
                "holds something the other does not. The host cannot tell which",
                "you want, and it does not guess: it has mounted no data and",
                "started nothing, and it waits here.",
                "",
                "Each unit is shown by its location code and serial number,",
                "with the time its disk was last written. Type 1 by the unit",
                "to keep and press Enter; the next screen asks you to confirm.",
                "",
                "The unit that is not kept is written again from the other.",
                "To look at a unit first, power the machine off and start it",
                "with that disk alone.",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::Runner;
    use scrkit::panel::End;

    struct Env {
        answers: bool,
        kept: Option<String>,
        refuse: bool,
    }

    impl Named for Env {
        fn system(&self) -> String {
            "SYSTEM1".into()
        }
    }

    impl Units for Env {
        fn apart(&mut self) -> Option<Vec<ApartUnit>> {
            let unit = |number: u32, disk: &str, written: &str| ApartUnit {
                number,
                disk: disk.into(),
                location: format!("P0000-B00-D1{}-F0", number - 1),
                serial: format!("DEMOHD{}", number - 1),
                written: written.into(),
            };
            self.answers.then(|| {
                vec![
                    unit(1, "vda", "2026-11-12 10:22 UTC"),
                    unit(2, "vdb", "2026-11-12 10:31 UTC"),
                ]
            })
        }

        fn keep(&mut self, disk: &str) -> Result<(), String> {
            if self.refuse {
                return Err("The label was not written.".into());
            }
            self.kept = Some(disk.into());
            Ok(())
        }
    }

    fn env() -> Env {
        Env {
            answers: true,
            kept: None,
            refuse: false,
        }
    }

    fn lines(r: &Runner<Env, u32>, env: &mut Env) -> Vec<String> {
        r.screen(env).dump().lines().map(str::to_string).collect()
    }

    fn fields(second: &str) -> Vec<String> {
        vec![String::new(), second.to_string()]
    }

    #[test]
    fn both_units_are_named_with_their_location_serial_and_time() {
        let mut env = env();
        let r: Runner<Env, u32> = Runner::new(Box::new(KeepUnit));
        let shown = lines(&r, &mut env);
        assert!(shown[0].contains(TITLE));
        assert!(shown[5].starts_with(" Opt Unit  Location"));
        assert!(shown[6].contains("1  P0000-B00-D10-F0"));
        assert!(
            shown[6]
                .trim_end()
                .ends_with("DEMOHD0             2026-11-12 10:22 UTC")
        );
        assert!(shown[7].contains("2  P0000-B00-D11-F0") && shown[7].contains("10:31 UTC"));
        assert!(shown[9].contains("started nothing"));
        assert_eq!(r.screen(&mut env).fields.len(), 2);
    }

    #[test]
    fn a_unit_is_kept_only_once_it_is_chosen_and_confirmed() {
        let mut env = env();
        let mut r: Runner<Env, u32> = Runner::new(Box::new(KeepUnit));
        // Nothing leaves the panel but a choice.
        assert_eq!(r.function(3, &[], &mut env), None);
        assert_eq!(r.function(12, &[], &mut env), None);
        assert!(lines(&r, &mut env)[23].contains("must be chosen"));
        assert_eq!(r.enter(&fields(""), &mut env), None);
        assert_eq!(r.enter(&["1".into(), "1".into()], &mut env), None);
        assert_eq!(r.top(), "keep-unit");
        // The second unit, then back, then again and confirmed.
        assert_eq!(r.enter(&fields("1"), &mut env), None);
        assert_eq!(r.top(), "keep-unit-confirm");
        let shown = lines(&r, &mut env);
        assert!(shown[5].contains("Unit 2  P0000-B00-D11-F0  serial DEMOHD1"));
        assert!(shown[11].contains("Unit 1  P0000-B00-D10-F0"));
        assert_eq!(r.function(12, &[], &mut env), None);
        assert_eq!(env.kept, None);
        r.enter(&fields("1"), &mut env);
        assert_eq!(r.function(10, &[], &mut env), Some(End::Outcome(2)));
        assert_eq!(env.kept.as_deref(), Some("vdb"));
    }

    #[test]
    fn a_refusal_is_shown_and_a_silent_service_is_asked_again() {
        let mut env = env();
        env.refuse = true;
        let mut r: Runner<Env, u32> = Runner::new(Box::new(KeepUnit));
        r.enter(&fields("1"), &mut env);
        assert_eq!(r.function(10, &[], &mut env), None);
        assert_eq!(r.top(), "keep-unit");
        assert!(lines(&r, &mut env)[23].contains("The label was not written."));
        env.answers = false;
        assert!(lines(&r, &mut env)[3].contains("does not answer"));
        assert_eq!(r.enter(&[String::new()], &mut env), None);
    }
}
