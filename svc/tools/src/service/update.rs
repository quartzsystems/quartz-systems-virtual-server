// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 8, 4: an update installed from a removable volume, then a
//! restart now or later; and option 8, 5: the code levels.

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use update::proto::{Attempt, Calls as _, Slot, Status, Verification};

use super::{down, said, volume, when};
use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

/// The update service's status, or why not.
fn status(env: &mut Env) -> Result<Status, String> {
    env.updates()
        .status()
        .map_err(|e| down("update service", &e))
}

fn attempt(a: Attempt) -> &'static str {
    match a {
        Attempt::Never => "never activated",
        Attempt::Pending => "pending",
        Attempt::Confirmed => "confirmed",
        Attempt::RolledBack => "rolled back",
    }
}

/// A slot's lines.
fn slot_lines(env: &Env, s: &Slot) -> Vec<String> {
    let line = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 20));
    let mut roles = Vec::new();
    if s.running {
        roles.push("running");
    }
    if s.default {
        roles.push("default");
    }
    let mut out = vec![
        line(
            &format!("Slot {}", s.name),
            if s.release.is_empty() {
                "unknown".into()
            } else {
                format!(
                    "{} build {}",
                    s.release,
                    if s.build.is_empty() { "-" } else { &s.build }
                )
            },
        ),
        line(
            "  Role",
            if roles.is_empty() {
                "-".into()
            } else {
                roles.join(", ")
            },
        ),
        line(
            "  Fixes",
            if s.fixes.is_empty() {
                "none".into()
            } else {
                s.fixes.join(" ")
            },
        ),
        line(
            "  Last activation",
            format!(
                "{}{}",
                attempt(s.last),
                if s.time == 0 {
                    String::new()
                } else {
                    format!(", {}", when(env, s.time))
                }
            ),
        ),
    ];
    if !s.reason.is_empty() {
        out.push(line("  Why", s.reason.clone()));
    }
    out
}

/// Option 8, 5: `Display Code Levels`.
pub struct Levels;

impl Panel<Env, Outcome> for Levels {
    fn name(&self) -> &'static str {
        "tools-levels"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let lines = match status(env) {
            Err(e) => vec![e],
            Ok(st) => {
                let mut out = Vec::new();
                for s in &st.slots {
                    out.extend(slot_lines(env, s));
                }
                let pending = st.slots.iter().find(|s| s.last == Attempt::Pending);
                out.push(format!(
                    "{} {}",
                    kit::dotted("Pending", 20),
                    pending.map_or_else(|| "none".to_string(), |s| format!("slot {}", s.name))
                ));
                out.push(format!(
                    "{} {}",
                    kit::dotted("Staged", 20),
                    st.staged.as_ref().map_or_else(
                        || "nothing".to_string(),
                        |s| s.image.as_ref().map_or_else(
                            || s.name.clone(),
                            |i| format!("{} build {} ({})", i.release, i.build, s.name)
                        )
                    )
                ));
                out
            }
        };
        kit::page(
            "Display Code Levels",
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
            title: "Display Code Levels",
            lines: &[
                "The host's code is in two slots. Each shows the release and",
                "build it holds, its fixes, whether the host runs from it",
                "and whether it is the default, and how its last activation",
                "ended: pending (installed, waiting for its restart),",
                "confirmed, or rolled back with why.",
            ],
        }
    }
}

/// Option 8, 4: `Install Update from Removable Volume`, the device.
pub struct FromVolume;

impl Panel<Env, Outcome> for FromVolume {
    fn name(&self) -> &'static str {
        "tools-update-device"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Install Update from Removable Volume",
            &env.system(),
            &[FormLine {
                label: "Device",
                len: 10,
                value: "sdb1".into(),
                hidden: false,
            }],
            "Enter=Continue   F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 2,
            "The update images (.upd files) on the volume are listed next.",
        );
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let device = fields.first().map_or("", |f| f.trim()).to_string();
        let files = match volume::mount(&device, false) {
            Ok(m) => volume::files(m.path(), "upd"),
            Err(e) => return Flow::Error(e),
        };
        if files.is_empty() {
            return Flow::Error(format!("{device} holds no update image (.upd file)."));
        }
        Flow::Push(Box::new(Choose { device, files }))
    }

    fn help(&self) -> Help {
        Help {
            title: "Install Update from Removable Volume",
            lines: &[
                "For a host no management console manages: an update image",
                "made by the release's build, on a removable volume.",
                "",
                "Device: the kernel's name of the volume (sdb1). It is",
                "mounted to read only, and unmounted when the image is",
                "copied.",
            ],
        }
    }
}

/// `Select Update Image`.
struct Choose {
    device: String,
    files: Vec<String>,
}

impl Panel<Env, Outcome> for Choose {
    fn name(&self) -> &'static str {
        "tools-update-choose"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let rows: Vec<String> = self.files.iter().map(|f| kit::fit(f, 70)).collect();
        ui::list(
            "Select Update Image",
            &env.system(),
            "1=Select  2=Select, allowing an older release",
            "Opt File",
            &rows,
            "Enter=Select  F1=Help  F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let chosen: Vec<(&String, u32)> = self
            .files
            .iter()
            .zip(fields)
            .filter_map(|(f, o)| Some((f, kit::number(Some(o))?)))
            .collect();
        let [(file, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one image.".into())
            };
        };
        let allow_older = match option {
            1 => false,
            2 => true,
            _ => return Flow::Error("Option not valid.".into()),
        };
        let mounted = match volume::mount(&self.device, false) {
            Ok(m) => m,
            Err(e) => return Flow::Error(e),
        };
        let Some(path) = volume::within(mounted.path(), file) else {
            return Flow::Error("The file is not on the volume.".into());
        };
        let staged = env
            .updates()
            .stage_file(path.display().to_string(), allow_older);
        drop(mounted);
        match staged {
            Ok(Ok(s)) if s.verification == Verification::Verified => Flow::Push(Box::new(Image)),
            Ok(Ok(s)) => Flow::Error(said(&s.error)),
            Ok(Err(issued)) => Flow::Error(said(&issued)),
            Err(e) => Flow::Error(down("update service", &e)),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Select Update Image",
            lines: &[
                "Type 1 by the image to install. It is copied to the host",
                "and verified: its signature by a key this release trusts,",
                "its hashes, its product, its release, and the state it",
                "reads. Type 2 to allow a release older than the one that",
                "runs.",
            ],
        }
    }
}

/// `Update Image`: what is staged beside what runs.
struct Image;

impl Panel<Env, Outcome> for Image {
    fn name(&self) -> &'static str {
        "tools-update-image"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let line = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 20));
        let lines = match status(env) {
            Err(e) => vec![e],
            Ok(st) => {
                let running = st.slots.iter().find(|s| s.running);
                let other = st.slots.iter().find(|s| !s.running);
                let mut out = Vec::new();
                if let Some(s) = st.staged.as_ref() {
                    let i = s.image.clone().unwrap_or_default();
                    out.push(line("Image", s.name.clone()));
                    out.push(line("Product", i.product));
                    out.push(line("Release", format!("{} build {}", i.release, i.build)));
                    out.push(line(
                        "Fixes",
                        if i.fixes.is_empty() {
                            "none".into()
                        } else {
                            i.fixes.join(" ")
                        },
                    ));
                    out.push(line("Size", kit::size(i.bytes)));
                    out.push(line(
                        "Verified",
                        "signature, hashes, product, release, state".into(),
                    ));
                }
                out.push(String::new());
                if let Some(r) = running {
                    out.push(line(
                        "Running",
                        format!("{} build {} in slot {}", r.release, r.build, r.name),
                    ));
                }
                if let Some(o) = other {
                    out.push(line(
                        "Written to",
                        format!("slot {} on every load-source disk", o.name),
                    ));
                }
                out.push(String::new());
                out.push("Press F10 to install it, or F12 to discard it.".into());
                out
            }
        };
        kit::page(
            "Update Image",
            &env.system(),
            &lines,
            "F1=Help   F10=Install   F12=Discard",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut Env) -> Step {
        match key {
            10 => match env.updates().install() {
                Ok(Ok(slot)) => Flow::Replace(Box::new(Installing { slot })),
                Ok(Err(issued)) => Flow::Error(said(&issued)),
                Err(e) => Flow::Error(down("update service", &e)),
            },
            3 | 12 => {
                let _ = env.updates().discard();
                Flow::Unwind(2, Some(("The update image is discarded.".into(), false)))
            }
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Update Image",
            lines: &[
                "The image verified. F10 writes it to the slot the host does",
                "not run from, on every load-source disk, reads it back, and",
                "marks the slot pending; nothing running is disturbed. F12",
                "discards the image.",
            ],
        }
    }
}

/// `Installing Update`: the progress, then restart now or later.
struct Installing {
    slot: String,
}

impl Panel<Env, Outcome> for Installing {
    fn name(&self) -> &'static str {
        "tools-update-installing"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut lines = Vec::new();
        let mut keys = "F1=Help   F5=Refresh   F12=Exit";
        match status(env) {
            Err(e) => lines.push(e),
            Ok(st) => {
                if let Some(p) = &st.installing {
                    lines.push(format!("Writing slot {}: {}%", p.slot, p.percent));
                    lines.push(String::new());
                    lines.push("F5 shows how far it is.".into());
                } else if st
                    .slots
                    .iter()
                    .any(|s| s.name == self.slot && s.last == Attempt::Pending)
                {
                    lines.push(format!(
                        "Slot {} is written on every load-source disk and pending.",
                        self.slot
                    ));
                    lines.push(String::new());
                    lines.push("Select one of the following:".into());
                    lines.push(String::new());
                    lines.push("    1. Restart now into the update".into());
                    lines.push("    2. Restart later".into());
                    // A choice to make: Enter makes it.
                    keys = "Enter=Select   F1=Help   F5=Refresh   F12=Exit";
                } else {
                    let why = st
                        .slots
                        .iter()
                        .find(|s| s.name == self.slot)
                        .map(|s| s.reason.clone())
                        .unwrap_or_default();
                    lines.push(format!("Slot {} was not installed. {why}", self.slot));
                    lines.push("The history says why (A2012030).".into());
                }
            }
        }
        let mut s = kit::form(
            "Installing Update",
            &env.system(),
            &[FormLine {
                label: "Selection",
                len: 1,
                value: String::new(),
                hidden: false,
            }],
            keys,
        );
        for (i, l) in lines.iter().enumerate() {
            kit::put(&mut s, ui::BODY_ROW + 2 + i, l);
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let pending = status(env).is_ok_and(|st| {
            st.installing.is_none()
                && st
                    .slots
                    .iter()
                    .any(|s| s.name == self.slot && s.last == Attempt::Pending)
        });
        match (kit::number(fields.first()), pending) {
            (_, false) => Flow::Stay,
            (Some(1), true) => Flow::Push(Box::new(RestartNow {
                slot: self.slot.clone(),
            })),
            (Some(2), true) => Flow::Unwind(
                3,
                Some((
                    format!(
                        "Slot {} is pending: option 8, 5 shows it; the restart is the update's.",
                        self.slot
                    ),
                    false,
                )),
            ),
            _ => Flow::Error("Selection not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            12 | 3 => Flow::Unwind(3, None),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Installing Update",
            lines: &[
                "The image is being written and read back. When the slot is",
                "pending: 1 restarts into it now, after the partitions are",
                "shut down; 2 leaves it pending for a restart later, by",
                "option 8, 4 again or a management console.",
            ],
        }
    }
}

/// What is typed to confirm the restart.
const RESTART_WORD: &str = "RESTART";

/// `Confirm Restart into Update`: the partitions that would be shut
/// down, their timeout, and a typed confirmation.
struct RestartNow {
    slot: String,
}

impl Panel<Env, Outcome> for RestartNow {
    fn name(&self) -> &'static str {
        "tools-update-restart"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let running: Vec<String> = crate::partitions::list()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.state.active())
            .map(|p| format!("  {:>3}  {}", p.id, p.name))
            .collect();
        let mut s = kit::form(
            "Confirm Restart into Update",
            &env.system(),
            &[
                FormLine {
                    label: "Seconds each partition has",
                    len: 5,
                    value: "300".into(),
                    hidden: false,
                },
                FormLine {
                    label: "Type RESTART to confirm",
                    len: 8,
                    value: String::new(),
                    hidden: false,
                },
            ],
            "Enter=Confirm   F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 3,
            &format!(
                "The host restarts into slot {}. First these partitions are",
                self.slot
            ),
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 4,
            "shut down through their operating systems:",
        );
        if running.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 5, "  (No partition is running.)");
        }
        for (row, l) in (ui::BODY_ROW + 5..).zip(running.iter().take(kit::BODY_ROWS - 7)) {
            kit::put(&mut s, row, l);
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let timeout = fields
            .first()
            .and_then(|f| f.trim().parse::<u32>().ok())
            .filter(|t| *t <= 99_999);
        let Some(timeout) = timeout else {
            return Flow::Error("Type the seconds as a number, 0 to 99999.".into());
        };
        if fields.get(1).map_or("", |f| f.trim()) != RESTART_WORD {
            return Flow::Error(format!(
                "Type {RESTART_WORD} to confirm, or F12 to go back."
            ));
        }
        let by = env.who();
        match env.updates().activate(true, timeout, by) {
            Ok(Ok(())) => Flow::Message(format!(
                "The host restarts into slot {} once the partitions are shut down.",
                self.slot
            )),
            Ok(Err(issued)) => Flow::Error(said(&issued)),
            Err(e) => Flow::Error(down("update service", &e)),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm Restart into Update",
            lines: &[
                "Each running partition is shut down through its operating",
                "system and given the seconds typed; one still running then",
                "is ended. The host then restarts into the pending slot. It",
                "confirms when its services are ready; if it does not, the",
                "host comes back on this slot with its state as it was.",
            ],
        }
    }
}
