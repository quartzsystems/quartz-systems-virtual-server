// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 4, `Management Console Pairing` (`docs/SERVICETOOLS.md`): the
//! consoles paired with this host, a pairing password, a console's
//! removal, and what the QSI host server listens on. Each asks the
//! server's local socket; nothing here keeps a pin or a password.

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use qsilocal::{Calls as _, Console, Password};

use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

const TITLE: &str = "Management Console Pairing";
const CONSOLES: &str = "Work with Paired Consoles";
const NO_SERVER: &str = "The QSI host server does not answer.";

fn asked<T>(r: Result<T, ipc::WireError>) -> Result<T, String> {
    r.map_err(|_| NO_SERVER.to_string())
}

/// `ABCD EFGH JKMN PQRS`: in fours, as it is read out.
#[must_use]
pub fn grouped(text: &str, size: usize) -> String {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(size)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The option 4 menu.
pub struct Pairing;

impl Panel<Env, Outcome> for Pairing {
    fn name(&self) -> &'static str {
        "tools-qsi"
    }

    fn render(&self, env: &mut Env) -> Screen {
        ui::menu(
            TITLE,
            &env.system(),
            &[
                (1, "Work with paired consoles"),
                (2, "Start pairing"),
                (3, "Display management interface"),
            ],
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        match kit::number(fields.first()) {
            Some(1) => Flow::Push(Box::new(Consoles)),
            Some(2) => match asked(env.qsi().start_pairing()) {
                Ok(Ok(password)) => Flow::Push(Box::new(StartPairing { password })),
                Ok(Err(why)) | Err(why) => Flow::Error(why),
            },
            Some(3) => Flow::Push(Box::new(Interface)),
            _ => Flow::Error("Selection not valid.".into()),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "A management console (QS/MC) manages this host over QSI, on",
                "TCP port 2300 of the management address, once it is paired.",
                "",
                "1 lists the consoles paired, and removes one. 2 makes a",
                "pairing password to type at the console: one pairing, for 15",
                "minutes and five attempts. 3 shows where the host listens",
                "and the fingerprint of its certificate.",
                "",
                "Two consoles may be paired at once.",
            ],
        }
    }
}

fn day(env: &Env, secs: u64) -> String {
    if secs == 0 {
        "-".into()
    } else {
        env.zone().display(i64::try_from(secs).unwrap_or(i64::MAX))
    }
}

/// One console's row.
fn row(env: &Env, c: &Console) -> String {
    format!(
        "{:<16}  {:<16}  {:<22}  {}",
        kit::fit(&c.name, 16),
        c.fingerprint.get(..16).unwrap_or(&c.fingerprint),
        day(env, c.last_contact),
        if c.connected { "connected" } else { "" }
    )
    .trim_end()
    .to_string()
}

/// `Work with Paired Consoles`.
pub struct Consoles;

impl Panel<Env, Outcome> for Consoles {
    fn name(&self) -> &'static str {
        "tools-qsi-consoles"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Ok(all) = env.qsi().consoles() else {
            return kit::page(
                CONSOLES,
                &env.system(),
                &[NO_SERVER.to_string()],
                "F1=Help   F3=Exit   F12=Cancel",
            );
        };
        let rows: Vec<String> = all.iter().map(|c| row(env, c)).collect();
        let mut s = ui::list(
            CONSOLES,
            &env.system(),
            "4=Remove  5=Display",
            "Opt Name              Fingerprint       Last contact",
            &rows,
            "F1=Help  F3=Exit  F5=Refresh  F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(
                &mut s,
                ui::BODY_ROW + 3,
                "(No console is paired. Option 2 of the menu pairs one.)",
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let Ok(all) = env.qsi().consoles() else {
            return Flow::Error(NO_SERVER.into());
        };
        let chosen: Vec<(Console, u32)> = all
            .into_iter()
            .zip(fields)
            .filter_map(|(c, f)| Some((c, kit::number(Some(f))?)))
            .collect();
        let [(console, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one console.".into())
            };
        };
        match option {
            4 => Flow::Push(Box::new(Remove {
                console: console.clone(),
            })),
            5 => Flow::Push(Box::new(Display {
                console: console.clone(),
            })),
            _ => Flow::Error("Option not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: CONSOLES,
            lines: &[
                "The consoles paired with this host: the name each gave when",
                "it paired, the start of its certificate's fingerprint, and",
                "when it last had a session.",
                "",
                "Type 5 by a console to see it whole, or 4 to remove it: it can",
                "connect no more, and a session it has now ends.",
            ],
        }
    }
}

/// `Display Paired Console`.
struct Display {
    console: Console,
}

impl Panel<Env, Outcome> for Display {
    fn name(&self) -> &'static str {
        "tools-qsi-console"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let c = &self.console;
        let line = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 20));
        let lines = vec![
            line("Name", c.name.clone()),
            line(
                "Fingerprint",
                grouped(c.fingerprint.get(..32).unwrap_or(""), 4),
            ),
            format!(
                "{:21} {}",
                "",
                grouped(c.fingerprint.get(32..).unwrap_or(""), 4)
            ),
            line("Paired", day(env, c.paired)),
            line("Last contact", day(env, c.last_contact)),
            line(
                "From",
                if c.address.is_empty() {
                    "-".into()
                } else {
                    c.address.clone()
                },
            ),
            line("Session now", if c.connected { "yes" } else { "no" }.into()),
        ];
        kit::page(
            "Display Paired Console",
            &env.system(),
            &lines,
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Paired Console",
            lines: &[
                "One paired console. Its fingerprint is the SHA-256 of its",
                "certificate: the console shows the same digits.",
            ],
        }
    }
}

/// `Confirm Remove of Console`: by its name, typed.
struct Remove {
    console: Console,
}

impl Panel<Env, Outcome> for Remove {
    fn name(&self) -> &'static str {
        "tools-qsi-remove"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Confirm Remove of Console",
            &env.system(),
            &[FormLine {
                label: "Type its name to confirm",
                len: 32,
                value: String::new(),
                hidden: false,
            }],
            "F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 2,
            &format!(
                "Console {} is removed: it can connect no more.",
                self.console.name
            ),
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 3,
            "Its session, if it has one, ends. Pair it again to use it.",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 5,
            &format!("To go on, type: {}", self.console.name),
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let name = &self.console.name;
        if fields.first().map_or("", |f| f.trim()) != name.trim() {
            return Flow::Error(format!("Type {name} to confirm, or F12 to go back."));
        }
        match asked(env.qsi().remove(self.console.fingerprint.clone())) {
            Ok(Ok(_)) => Flow::Unwind(1, Some((format!("Console {name} is removed."), false))),
            Ok(Err(why)) | Err(why) => Flow::Error(why),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm Remove of Console",
            lines: &[
                "The console is removed when its name is typed in full and",
                "Enter is pressed. F12 goes back and nothing is done.",
            ],
        }
    }
}

/// `Start Pairing`: the password, shown here once, until it is used,
/// expires or is ended.
pub struct StartPairing {
    password: Password,
}

impl Panel<Env, Outcome> for StartPairing {
    fn name(&self) -> &'static str {
        "tools-qsi-pair"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let out = asked(env.qsi().pairing()).ok().flatten();
        let fingerprint = asked(env.qsi().interface())
            .map(|i| i.fingerprint)
            .unwrap_or_default();
        let line = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 20));
        let mut lines = vec![
            "Type this password at the management console to pair it:".to_string(),
            String::new(),
        ];
        if let Some(p) = out {
            // Already in groups, as QS/400's crate makes it.
            lines.push(format!("      {}", self.password.text));
            lines.push(String::new());
            lines.push(line(
                "Expires",
                env.zone()
                    .display(i64::try_from(p.until).unwrap_or(i64::MAX)),
            ));
            lines.push(line("Attempts left", p.attempts.to_string()));
        } else {
            lines.push("      The password is gone: a console paired with it, it".into());
            lines.push("      expired, or its attempts ran out.".into());
            lines.push(String::new());
            lines.push(String::new());
        }
        lines.push(String::new());
        lines.push("The console shows the host's fingerprint. It must be this one:".into());
        lines.push(format!(
            "      {}",
            grouped(fingerprint.get(..32).unwrap_or(""), 4)
        ));
        lines.push(format!(
            "      {}",
            grouped(fingerprint.get(32..).unwrap_or(""), 4)
        ));
        lines.push(String::new());
        lines.push("The password is good for one pairing. It is shown here only.".into());
        kit::page(
            "Start Pairing",
            &env.system(),
            &lines,
            "F1=Help   F5=Refresh   F12=End pairing",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut Env) -> Step {
        match key {
            3 | 12 => {
                let still = asked(env.qsi().pairing()).ok().flatten().is_some();
                if still {
                    let _ = env.qsi().end_pairing();
                    Flow::Unwind(
                        1,
                        Some(("Pairing ended: the password is gone.".into(), false)),
                    )
                } else {
                    Flow::Pop
                }
            }
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Start Pairing",
            lines: &[
                "At the management console, add this host by its management",
                "address and type the password. Both ends check it without",
                "sending it, and each pins the other's certificate.",
                "",
                "The console shows this host's fingerprint: check that it is",
                "the one here before you go on there.",
                "",
                "F5 shows the attempts left. F12 ends pairing: the password is",
                "gone. Leaving this panel ends it too.",
            ],
        }
    }
}

/// `Display Management Interface`.
pub struct Interface;

impl Panel<Env, Outcome> for Interface {
    fn name(&self) -> &'static str {
        "tools-qsi-interface"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let line = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 20));
        let lines = match asked(env.qsi().interface()) {
            Err(e) => vec![e],
            Ok(i) => {
                let mut lines = vec![line(
                    "Listening on",
                    if i.addresses.is_empty() {
                        "nothing".into()
                    } else {
                        i.addresses.join("  ")
                    },
                )];
                if !i.problem.is_empty() {
                    lines.push(format!("{:21} {}", "", i.problem));
                }
                lines.push(line("Protocol", "QSI version 1, TLS 1.3, mutual".into()));
                lines.push(line(
                    "Certificate",
                    grouped(i.fingerprint.get(..32).unwrap_or(""), 4),
                ));
                lines.push(format!(
                    "{:21} {}",
                    "",
                    grouped(i.fingerprint.get(32..).unwrap_or(""), 4)
                ));
                lines.push(String::new());
                lines.push(line("Sessions", i.sessions.len().to_string()));
                for (address, who) in i.sessions.iter().take(6) {
                    lines.push(format!("{:21} {address}  {who}", ""));
                }
                lines
            }
        };
        kit::page(
            "Display Management Interface",
            &env.system(),
            &lines,
            "F1=Help   F3=Exit   F5=Refresh   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Management Interface",
            lines: &[
                "Where the QSI host server listens: TCP port 2300 of each",
                "management address, and nothing else. A console connects",
                "there and checks the host's certificate by its fingerprint.",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_and_a_fingerprint_are_read_in_fours() {
        assert_eq!(grouped("ABCDEFGHJKMNPQRS", 4), "ABCD EFGH JKMN PQRS");
        assert_eq!(grouped("abcdef", 4), "abcd ef");
    }
}
