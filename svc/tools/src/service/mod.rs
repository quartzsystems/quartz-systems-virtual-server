// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 8, Service functions (`docs/SERVICETOOLS.md`): problems, the
//! host's dump, dumps kept, an update from a removable volume, the code
//! levels, and every partition shut down: what a host with no
//! management console needs.

pub mod dumps;
pub mod problems;
pub mod update;
pub mod volume;

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};

use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

/// The menu's title.
pub const TITLE: &str = "Service Functions";

/// A service that did not answer, said as a person reads it.
pub(crate) fn down(who: &str, e: &dyn std::fmt::Display) -> String {
    format!("The {who} does not answer: {e}.")
}

/// The first message of a refusal, its text.
pub(crate) fn said(issued: &[qvsmsg::Issued]) -> String {
    issued
        .first()
        .map_or_else(|| "It was refused.".to_string(), |i| i.text.clone())
}

/// A time as the display zone shows it; `-` for none.
pub(crate) fn when(env: &Env, secs: u64) -> String {
    if secs == 0 {
        "-".into()
    } else {
        env.zone().display(i64::try_from(secs).unwrap_or(i64::MAX))
    }
}

/// Option 8's menu.
pub struct ServiceMenu;

impl Panel<Env, Outcome> for ServiceMenu {
    fn name(&self) -> &'static str {
        "tools-service"
    }

    fn render(&self, env: &mut Env) -> Screen {
        ui::menu(
            TITLE,
            &env.system(),
            &[
                (1, "Work with problems"),
                (2, "Take a host dump"),
                (3, "Work with dumps"),
                (4, "Install an update from a removable volume"),
                (5, "Display code levels"),
                (6, "Shut down all partitions"),
            ],
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        match kit::number(fields.first()) {
            Some(1) => Flow::Push(Box::new(problems::Problems::default())),
            Some(2) => Flow::Push(Box::new(TakeDump)),
            Some(3) => Flow::Push(Box::new(dumps::Dumps::default())),
            Some(4) => Flow::Push(Box::new(update::FromVolume)),
            Some(5) => Flow::Push(Box::new(update::Levels)),
            Some(6) => Flow::Push(Box::new(EndAll)),
            _ => Flow::Error("Selection not valid.".into()),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "1 lists the host's problems: each condition a person should",
                "look at, with what it is about and what to do; one is closed",
                "here. 2 takes a host dump: the host's service data for the",
                "release's supplier. 3 lists the host's dumps and the",
                "partitions', copies one to a removable volume or deletes it.",
                "4 installs an update image from a removable volume into the",
                "other slot. 5 shows the code in each slot. 6 shuts every",
                "partition down through its operating system.",
            ],
        }
    }
}

/// `Take Host Dump`.
struct TakeDump;

impl Panel<Env, Outcome> for TakeDump {
    fn name(&self) -> &'static str {
        "tools-service-dump"
    }

    fn render(&self, env: &mut Env) -> Screen {
        kit::page(
            "Take Host Dump",
            &env.system(),
            &kit::lines(&[
                "A host dump holds the host's service data: its logs, the",
                "reference code and problem histories, the inventory, the",
                "state files, each service's status, the partitions, pools",
                "and switches, the control groups and the kernel's log.",
                "",
                "No secret goes in: not the host's key, not a password hash,",
                "not a console's pin.",
                "",
                "Press Enter to take one, or F12 to go back.",
            ]),
            "F1=Help   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], env: &mut Env) -> Step {
        let by = env.who();
        match env.take_dump(by) {
            Ok(name) => Flow::Unwind(
                1,
                Some((
                    format!(
                        "Host dump {name} is being taken; option 3 lists it once it is written."
                    ),
                    false,
                )),
            ),
            Err(why) => Flow::Error(why),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Take Host Dump",
            lines: &[
                "The dump is written while the host runs, into /data. Option",
                "3 lists it when it is written, and copies it to a removable",
                "volume to be taken to the release's supplier.",
            ],
        }
    }
}

/// `Shut Down All Partitions`: the list, and a typed confirmation.
struct EndAll;

/// What is typed to confirm.
const END_WORD: &str = "SHUTDOWN";

impl Panel<Env, Outcome> for EndAll {
    fn name(&self) -> &'static str {
        "tools-service-end"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let running: Vec<String> = crate::partitions::list()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.state.active())
            .map(|p| format!("  {:>3}  {}", p.id, p.name))
            .collect();
        let mut s = kit::form(
            "Shut Down All Partitions",
            &env.system(),
            &[FormLine {
                label: "Type SHUTDOWN to confirm",
                len: 10,
                value: String::new(),
                hidden: false,
            }],
            "F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 2,
            "Each partition is shut down through its operating system:",
        );
        if running.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "  (No partition is running.)");
        }
        for (row, line) in (ui::BODY_ROW + 3..).zip(running.iter().take(kit::BODY_ROWS - 5)) {
            kit::put(&mut s, row, line);
        }
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        use partd::Calls as _;
        if fields.first().map_or("", |f| f.trim()) != END_WORD {
            return Flow::Error(format!("Type {END_WORD} to confirm, or F12 to go back."));
        }
        match partd::Client::new(partd::proto::socket(release::RUN_DIR)).end_all() {
            Ok(0) => Flow::Unwind(1, Some(("No partition was running.".into(), false))),
            Ok(n) => Flow::Unwind(
                1,
                Some((
                    format!(
                        "{n} partition(s) are being shut down through their operating systems."
                    ),
                    false,
                )),
            ),
            Err(e) => Flow::Error(down("partition manager", &e)),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Shut Down All Partitions",
            lines: &[
                "Every running partition is asked to shut down through its",
                "operating system, in the reverse of the order they were",
                "activated in; one that does not end in its time is ended.",
                "Type SHUTDOWN and press Enter. F12 goes back and nothing is",
                "done.",
            ],
        }
    }
}
