// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 8, 3: the host's dumps and the partitions', each copied to a
//! removable volume or deleted.

use std::fs;
use std::path::Path;

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use partd::Calls as _;

use super::{down, said, volume, when};
use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

const TITLE: &str = "Work with Dumps";
const ROWS: usize = kit::BODY_ROWS - 4;

/// One dump: the host's, or a partition's.
#[derive(Clone, Debug)]
pub struct Dump {
    /// The partition's ID and name; none for the host's own.
    pub partition: Option<(u32, String)>,
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// When it was taken.
    pub time: u64,
}

fn partd() -> partd::Client {
    partd::Client::new(partd::proto::socket(release::RUN_DIR))
}

fn host_store() -> hostdump::Store {
    hostdump::store::at(Path::new(release::DATA_DIR).join(hostdump::store::DIR))
}

/// Every dump: the host's, then each partition's, oldest first.
fn all() -> Vec<Dump> {
    let mut out: Vec<Dump> = host_store()
        .list()
        .into_iter()
        .map(|i| Dump {
            partition: None,
            name: i.name,
            bytes: i.bytes,
            time: i.time,
        })
        .collect();
    let mut c = partd();
    for p in c.list().unwrap_or_default() {
        for d in c.dumps(p.id).ok().and_then(Result::ok).unwrap_or_default() {
            out.push(Dump {
                partition: Some((p.id, p.name.clone())),
                name: d.name,
                bytes: d.bytes,
                time: d.time,
            });
        }
    }
    out
}

fn whose(d: &Dump) -> String {
    d.partition
        .as_ref()
        .map_or_else(|| "host".to_string(), |(_, n)| n.clone())
}

/// `Work with Dumps`.
#[derive(Default)]
pub struct Dumps {
    top: usize,
    /// Rows at the last draw: a page key needs not make the list again.
    seen: std::cell::Cell<usize>,
}

impl Panel<Env, Outcome> for Dumps {
    fn name(&self) -> &'static str {
        "tools-dumps"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let list = all();
        let rows: Vec<String> = list
            .iter()
            .map(|d| {
                format!(
                    "{}  {}  {:>9}  {}",
                    kit::fit(&whose(d), 10),
                    kit::fit(&d.name, 30),
                    kit::size(d.bytes),
                    when(env, d.time)
                )
            })
            .collect();
        self.seen.set(rows.len());
        let (shown, foot) = kit::window(&rows, self.top, ROWS);
        let mut s = ui::list(
            TITLE,
            &env.system(),
            "4=Delete  6=Copy to removable volume",
            "Opt Whose       Name                                 Size  Taken",
            &shown,
            "Enter=Select  F1=Help  F3=Exit  F5=Refresh  F12=Cancel",
        );
        kit::put(&mut s, ui::BODY_ROW + 4 + ROWS, &foot);
        if rows.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(No dump is kept.)");
        }
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let list = all();
        let chosen: Vec<(&Dump, u32)> = list
            .iter()
            .skip(self.top)
            .take(ROWS)
            .zip(fields)
            .filter_map(|(d, f)| Some((d, kit::number(Some(f))?)))
            .collect();
        let [(d, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one dump.".into())
            };
        };
        match option {
            4 => Flow::Push(Box::new(Delete { dump: (*d).clone() })),
            6 => Flow::Push(Box::new(Copy { dump: (*d).clone() })),
            _ => Flow::Error("Option not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            7 | 8 => {
                self.top = kit::scroll(self.top, key, self.seen.get(), ROWS);
                Flow::Stay
            }
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "The dumps kept: the host's own (service data, for the",
                "release's supplier) and each partition's (its memory, for",
                "the system's supplier), with their sizes and when they were",
                "taken.",
                "",
                "Type 6 by one to copy it to a removable volume, 4 to delete",
                "it.",
            ],
        }
    }
}

/// `Copy Dump to Removable Volume`.
struct Copy {
    dump: Dump,
}

/// Copy `d` to the volume mounted at `root`: its file's path there.
fn copy(d: &Dump, root: &Path) -> Result<String, String> {
    let name = match &d.partition {
        None => d.name.clone(),
        Some((_, partition)) => format!("{partition}-{}", d.name),
    };
    let target = root.join(&name);
    match &d.partition {
        None => {
            let from = host_store()
                .path(&d.name)
                .ok_or_else(|| format!("{} is no dump's name.", d.name))?;
            fs::copy(&from, &target).map_err(|e| format!("{name}: {e}"))?;
        }
        Some((id, _)) => {
            let mut out = fs::File::create(&target).map_err(|e| format!("{name}: {e}"))?;
            match partd().read_dump_into(*id, &d.name, &mut out) {
                Ok(Ok(_)) => {}
                Ok(Err(issued)) => {
                    let _ = fs::remove_file(&target);
                    return Err(said(&issued));
                }
                Err(e) => {
                    let _ = fs::remove_file(&target);
                    return Err(down("partition manager", &e));
                }
            }
        }
    }
    fs::File::open(&target)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(name)
}

impl Panel<Env, Outcome> for Copy {
    fn name(&self) -> &'static str {
        "tools-dump-copy"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Copy Dump to Removable Volume",
            &env.system(),
            &[FormLine {
                label: "Device",
                len: 10,
                value: "sdb1".into(),
                hidden: false,
            }],
            "Enter=Apply   F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 2,
            &format!(
                "{} dump {}, {}",
                whose(&self.dump),
                self.dump.name,
                kit::size(self.dump.bytes)
            ),
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 3,
            "is copied to the top of the volume on that device.",
        );
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let device = fields.first().map_or("", |f| f.trim());
        let mounted = match volume::mount(device, true) {
            Ok(m) => m,
            Err(e) => return Flow::Error(e),
        };
        match copy(&self.dump, mounted.path()) {
            Ok(name) => {
                drop(mounted);
                Flow::Unwind(1, Some((format!("{name} is copied to {device}."), false)))
            }
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Copy Dump to Removable Volume",
            lines: &[
                "Device: the kernel's name of a removable volume of the host,",
                "a FAT, exFAT, ext4 or xfs file system: sdb1. The dump is",
                "written to its top directory (a partition's named after the",
                "partition) and the volume is unmounted after.",
            ],
        }
    }
}

/// `Confirm Delete of Dump`.
struct Delete {
    dump: Dump,
}

impl Panel<Env, Outcome> for Delete {
    fn name(&self) -> &'static str {
        "tools-dump-delete"
    }

    fn render(&self, env: &mut Env) -> Screen {
        kit::page(
            "Confirm Delete of Dump",
            &env.system(),
            &[
                format!("{} dump {} is deleted.", whose(&self.dump), self.dump.name),
                String::new(),
                "Press Enter to delete it, or F12 to go back.".into(),
            ],
            "Enter=Confirm   F1=Help   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], env: &mut Env) -> Step {
        let done = match &self.dump.partition {
            None => host_store().delete(&self.dump.name).map(|()| {
                let detail = format!("{}, by {}", self.dump.name, env.who());
                env.post(&refcode::HOST_DUMP_REMOVED, &detail);
            }),
            Some((id, _)) => match partd().delete_dump(*id, self.dump.name.clone()) {
                Ok(Ok(())) => Ok(()),
                Ok(Err(issued)) => Err(said(&issued)),
                Err(e) => Err(down("partition manager", &e)),
            },
        };
        match done {
            Ok(()) => Flow::Unwind(1, Some((format!("{} is deleted.", self.dump.name), false))),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm Delete of Dump",
            lines: &["The dump is gone once it is deleted: copy it first if it is needed."],
        }
    }
}
