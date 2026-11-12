// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's own panels: the main menu, the system status, and the idle
//! screen a console shows while nobody is signed on.

use console::{Attr, Screen};
use panels::tools::{Log, LogPanel, NetworkPanel, TimePanel, UsersPanel, power};
use panels::ui;
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use release::fact;
use supervise::control::HostStatus;

use crate::disks::DiskUnits;
use crate::env::Env;

/// The title of the console.
pub const TITLE: &str = "QS/VS Service Tools";

/// How a session ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The user signed off.
    SignOff,
}

type Step = PanelFlow<Env, Outcome>;

/// The options that are not there yet, and what brings them.
const LATER: [(u8, &str); 2] = [
    (
        4,
        "Management console pairing comes with QSI, in a later phase.",
    ),
    (
        8,
        "Service functions come with code update, in a later phase.",
    ),
];

/// The main menu.
pub struct MainMenu;

impl Panel<Env, Outcome> for MainMenu {
    fn name(&self) -> &'static str {
        "tools-main"
    }

    fn render(&self, env: &mut Env) -> Screen {
        ui::menu(
            TITLE,
            &env.system(),
            &[
                (1, "Work with system status"),
                (2, "Work with disk units"),
                (3, "Configure management network"),
                (
                    4,
                    "Management console pairing  (not yet available: comes with QSI)",
                ),
                (5, "Work with service tools user IDs"),
                (6, "Date and time"),
                (7, "Display logs"),
                (
                    8,
                    "Service functions  (not yet available: comes with code update)",
                ),
                (9, "Power off or restart"),
            ],
            "F1=Help   F3=Sign off   F12=Sign off",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        match kit::number(fields.first()) {
            Some(1) => Flow::Push(Box::new(SystemStatus)),
            Some(2) => Flow::Push(Box::new(DiskUnits)),
            Some(3) => Flow::Push(Box::new(NetworkPanel::new(env))),
            Some(5) => Flow::Push(Box::new(UsersPanel)),
            Some(6) => Flow::Push(Box::new(TimePanel::new(env))),
            Some(7) => Flow::Push(Box::new(Choice {
                name: "tools-logs",
                title: "Display Logs",
                options: [(1, "Reference code history"), (2, "System log")],
                help: Help {
                    title: "Display Logs",
                    lines: &[
                        "1 shows the reference code history: every code the host",
                        "has posted, with its time, kept across IPLs.",
                        "",
                        "2 shows the system log: what PID 1 and the services say.",
                    ],
                },
            })),
            Some(9) => Flow::Push(Box::new(Choice {
                name: "tools-power",
                title: "Power Off or Restart",
                options: [(1, "Power off"), (2, "Restart")],
                help: Help {
                    title: "Power Off or Restart",
                    lines: &[
                        "1 shuts the host down in order and powers it off.",
                        "2 shuts it down in order and restarts it.",
                        "",
                        "Either asks you to confirm first. Partitions that run on",
                        "the host end with it.",
                    ],
                },
            })),
            Some(n) => match LATER.iter().find(|(option, _)| u32::from(*option) == n) {
                Some((_, why)) => Flow::Error(format!("Option {n} is not available yet. {why}")),
                None => Flow::Error("Selection not valid.".into()),
            },
            None => Flow::Error("Type the number of an option.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 | 12 => Flow::Exit(Outcome::SignOff),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "Type the number of an option and press Enter.",
                "",
                "1 shows the host's state. 2 shows the load-source disks and",
                "their protection, and adds or replaces one. 3 changes the",
                "management network. 5 works with the users of this console.",
                "6 sets the display zone, the time servers and the clock.",
                "7 shows the logs. 9 powers the host off or restarts it.",
                "",
                "Options 4 and 8 are not available in this release. Partitions",
                "are made and run from the management console, QS/MC.",
                "",
                "F3 or F12 signs you off. So does ten minutes without a key.",
            ],
        }
    }
}

/// A menu of two that leads to the logs or to the power confirmations.
struct Choice {
    name: &'static str,
    title: &'static str,
    options: [(u8, &'static str); 2],
    help: Help,
}

impl Panel<Env, Outcome> for Choice {
    fn name(&self) -> &'static str {
        self.name
    }

    fn render(&self, env: &mut Env) -> Screen {
        ui::menu(
            self.title,
            &env.system(),
            &self.options,
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let logs = self.name == "tools-logs";
        match (kit::number(fields.first()), logs) {
            (Some(1), true) => Flow::Push(Box::new(LogPanel::new(Log::History))),
            (Some(2), true) => Flow::Push(Box::new(LogPanel::new(Log::System))),
            (Some(1), false) => Flow::Push(Box::new(power::<Env, Outcome>(false))),
            (Some(2), false) => Flow::Push(Box::new(power::<Env, Outcome>(true))),
            _ => Flow::Error("Selection not valid.".into()),
        }
    }

    fn help(&self) -> Help {
        self.help
    }
}

/// `0 days 00:03:12` from the kernel's uptime.
fn uptime() -> String {
    let secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|t| t.split('.').next().and_then(|s| s.parse::<u64>().ok()))
        .unwrap_or(0);
    format!(
        "{} days {:02}:{:02}:{:02}",
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// The lines of the system status.
pub fn status_lines(status: &HostStatus, uptime: &str) -> Vec<String> {
    let of = |key: &str| status.fact(key).unwrap_or("").to_string();
    let row = |label: &str, value: &str| format!("{} {value}", kit::dotted(label, 19));
    let state = if of(fact::READY) == "yes" {
        "Running; every service has been ready"
    } else {
        "IPL in progress"
    };
    let mut lines = vec![
        row("Host state", state),
        row(
            "Release",
            &format!(
                "{} build {} ({} image)",
                of(fact::RELEASE),
                of(fact::BUILD),
                of(fact::IMAGE)
            ),
        ),
        row(
            "Slot",
            &format!("{} ({})", of(fact::SLOT), of(fact::RECORD)),
        ),
        row("Reference code", &kit::fit(&of(fact::REFERENCE), 56)),
        row("Management", &kit::fit(&of(fact::MANAGEMENT), 56)),
        row("Data", &of(fact::DATA)),
        row("KVM", &kit::fit(&of(fact::KVM), 56)),
        row("Uptime", uptime),
        String::new(),
        "Services:".to_string(),
    ];
    // Two to a line, so a long table stays on the screen.
    for pair in status.services.chunks(2) {
        let cells: Vec<String> = pair
            .iter()
            .map(|s| {
                let failures = match s.failures {
                    0 => String::new(),
                    n => format!(" ({n} failed)"),
                };
                kit::fit(&format!("{:<8} {}{failures}", s.name, s.state), 36)
            })
            .collect();
        lines.push(format!("  {}", cells.join(" ")).trim_end().to_string());
    }
    lines.push(String::new());
    lines.push("Partitions are managed from the management console, QS/MC.".into());
    lines
}

/// `Work with system status`.
struct SystemStatus;

impl Panel<Env, Outcome> for SystemStatus {
    fn name(&self) -> &'static str {
        "tools-status"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let status = env.status();
        kit::page(
            "Work with System Status",
            &env.system(),
            &status_lines(&status, &uptime()),
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn help(&self) -> Help {
        Help {
            title: "Work with System Status",
            lines: &[
                "The host as it is now; the display follows it.",
                "",
                "Host state says whether every service has been ready since the",
                "IPL. Slot is the slot this IPL came from, with the A/B record:",
                "the default slot and any slot on trial. Reference code is the",
                "code posted last. Management is the management address, or why",
                "there is none. Data is what the host's own file system is",
                "mounted from. KVM says whether partitions can run here.",
                "",
                "Services lists each service of the host with its state, and",
                "how often it has failed if it has.",
            ],
        }
    }

    fn live(&self) -> bool {
        true
    }
}

/// The idle screen: the host's status, and how to sign on.
pub fn idle(status: &HostStatus) -> Screen {
    let mut s = Screen::new();
    let mut lines = release::screen::lines(&release::banner(), status);
    // The last line is the one below, in its own place.
    lines.pop();
    for (i, line) in lines.iter().take(20).enumerate() {
        let attr = if i == 0 { Attr::WHITE } else { Attr::GREEN };
        s.put(
            1 + i + usize::from(i > 0),
            ui::BODY_COL,
            &kit::fit(line, 78),
            attr,
        );
    }
    s.put(
        ui::KEYS_ROW,
        ui::BODY_COL,
        release::screen::SIGN_ON,
        Attr::WHITE,
    );
    kit::enterable(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use supervise::control::ServiceStatus;

    fn status() -> HostStatus {
        let f = |k: &str, v: &str| (k.to_string(), v.to_string());
        HostStatus {
            facts: vec![
                f(fact::RELEASE, "V1R0M0"),
                f(fact::BUILD, "abc1234"),
                f(fact::IMAGE, "release"),
                f(fact::SLOT, "A"),
                f(fact::RECORD, "default A, nothing pending"),
                f(fact::HOST, "vs-01"),
                f(fact::MANAGEMENT, "10.0.2.15/24 on eth0, gateway 10.0.2.2"),
                f(fact::DATE, "2026-11-12 14:05 UTC"),
                f(
                    fact::REFERENCE,
                    "C2001090 Base ready: 0.85 s after the kernel started",
                ),
                f(fact::DATA, "/dev/ublkb0"),
                f(fact::READY, "yes"),
                f(fact::KVM, "KVM usable: VMX, API version 12"),
            ],
            services: ["stgd", "logd", "netd"]
                .iter()
                .map(|name| ServiceStatus {
                    name: (*name).to_string(),
                    state: "ready".into(),
                    pid: 5,
                    failures: u32::from(*name == "netd"),
                })
                .collect(),
        }
    }

    #[test]
    fn the_system_status_names_every_fact_and_service() {
        let lines = status_lines(&status(), "0 days 00:03:12");
        assert_eq!(
            lines[0],
            "Host state . . . . : Running; every service has been ready"
        );
        assert_eq!(
            lines[1],
            "Release  . . . . . : V1R0M0 build abc1234 (release image)"
        );
        assert_eq!(
            lines[2],
            "Slot . . . . . . . : A (default A, nothing pending)"
        );
        assert_eq!(lines[7], "Uptime . . . . . . : 0 days 00:03:12");
        assert!(lines[10].starts_with("  stgd     ready"));
        assert!(lines[10].contains("logd     ready"));
        assert_eq!(lines[11], "  netd     ready (1 failed)");
        assert!(lines.last().unwrap().contains("QS/MC"));
        // It fits a text display.
        assert!(lines.len() <= 16);
    }

    #[test]
    fn the_idle_screen_is_the_status_screen_and_takes_enter() {
        let s = idle(&status());
        let dump = s.dump();
        let rows: Vec<&str> = dump.lines().collect();
        assert!(rows[0].contains(&release::banner()));
        assert!(rows[2].starts_with(" Slot           A (default A, nothing pending)"));
        assert!(dump.contains("Service        netd       ready, 1 failure"));
        assert!(rows[22].contains("Press Enter to sign on to Service Tools."));
        assert_eq!(s.fields.len(), 1);
    }

    #[test]
    fn options_that_are_not_there_yet_say_what_brings_them() {
        assert!(LATER.iter().any(|(n, why)| *n == 4 && why.contains("QSI")));
        assert!(LATER.iter().any(|(n, _)| *n == 8));
    }
}
