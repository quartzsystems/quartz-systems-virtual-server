// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 2's storage beside the load source: the host's disks and
//! what each is, the storage pools (made, watched, a unit replaced, hot
//! spares added and taken away, renamed, deleted, the media repository
//! designated, an image imported), and, to be read only, the virtual
//! disks and the media library, which are managed from QS/MC. The pool
//! service does the work; these panels ask it and show what it says.

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use poold::proto::{Calls as _, Disk, PoolStatus, Refused};

use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

const POOLS: &str = "Work with Storage Pools";
const MANAGED: &str = "Virtual disks, media and tape volumes are managed from QS/MC.";

/// The answer of the pool service as a panel takes it.
fn asked<T>(answer: Result<Result<T, Refused>, ipc::WireError>) -> Result<T, String> {
    match answer {
        Err(e) => Err(format!("The pool service does not answer: {e}.")),
        Ok(Err(refused)) => Err(refused.first().map_or_else(
            || "The request was refused.".to_string(),
            ToString::to_string,
        )),
        Ok(Ok(v)) => Ok(v),
    }
}

fn disks(env: &mut Env) -> Vec<Disk> {
    env.pools().disks().unwrap_or_default()
}

fn pools(env: &mut Env) -> Option<Vec<PoolStatus>> {
    env.pools().pools().ok()
}

fn pool(env: &mut Env, name: &str) -> Option<PoolStatus> {
    pools(env)?.into_iter().find(|p| p.name == name)
}

fn percent(part: u64, whole: u64) -> String {
    (part * 100)
        .checked_div(whole)
        .map_or_else(String::new, |p| format!("{p}%"))
}

fn no_service(title: &str, env: &mut Env) -> Screen {
    kit::page(
        title,
        &env.system(),
        &kit::lines(&["The pool service does not answer."]),
        "F3=Exit   F12=Cancel",
    )
}

/// A disk as a row: location, serial, size, what it is.
fn disk_row(d: &Disk) -> String {
    format!(
        "{} {} {:>9}  {}",
        kit::fit(&d.location, 20),
        kit::fit(&d.serial, 16),
        kit::size(d.bytes),
        kit::fit(&d.what, 24)
    )
}

/// `Display disk configuration`.
pub struct DiskConfig;

impl Panel<Env, Outcome> for DiskConfig {
    fn name(&self) -> &'static str {
        "tools-disk-config"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let rows: Vec<String> = disks(env).iter().map(disk_row).collect();
        let mut text =
            vec!["Location             Serial                Size  What it is".to_string()];
        if rows.is_empty() {
            text.push("(The pool service does not answer, or the host has no disk.)".into());
        }
        text.extend(rows);
        kit::page(
            "Display Disk Configuration",
            &env.system(),
            &text,
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Disk Configuration",
            lines: &[
                "Every disk of the host, by its location code, serial number and",
                "size, and what it is: the load source, a unit or a hot spare of",
                "one of this host's storage pools with the unit's state, not",
                "configured, or foreign.",
                "",
                "A foreign disk holds what this host did not put there: another",
                "host's pool, a partition table, a file system. Nothing is taken",
                "from it unless its name is typed to confirm that it is erased.",
            ],
        }
    }
}

/// `Work with storage pools`.
pub struct Pools;

impl Pools {
    fn row(p: &PoolStatus) -> String {
        let state = if p.work.is_empty() {
            p.state.clone()
        } else {
            format!("{} {}%", p.state, p.percent)
        };
        let mark = if p.repository { "*" } else { " " };
        format!(
            "{}{} {} {} {:>9} {:>5}",
            kit::fit(&p.name, 10),
            mark,
            kit::fit(&p.protection, 9),
            kit::fit(&state, 19),
            kit::size(p.bytes),
            percent(p.allocated, p.bytes)
        )
    }
}

impl Panel<Env, Outcome> for Pools {
    fn name(&self) -> &'static str {
        "tools-pools"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Some(list) = pools(env) else {
            return no_service(POOLS, env);
        };
        let rows: Vec<String> = list.iter().map(Self::row).collect();
        let mut s = ui::list(
            POOLS,
            &env.system(),
            "2=Rename 4=Delete 5=Display 6=Replace unit 7=Add spare 8=Remove spare 9=Media",
            "Opt Name        Protection State                Size  Used",
            &rows,
            "F1=Help F3=Exit F5=Refresh F6=Create F7=Import image F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(No storage pool. F6 makes one.)");
        }
        kit::put(
            &mut s,
            ui::BODY_ROW + 3 + rows.len().max(1) + 1,
            "* holds the media library",
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let Some(list) = pools(env) else {
            return Flow::Error("The pool service does not answer.".into());
        };
        let chosen: Vec<(&PoolStatus, u32)> = list
            .iter()
            .zip(fields)
            .filter_map(|(p, f)| kit::number(Some(f)).map(|n| (p, n)))
            .collect();
        let [(p, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one pool.".into())
            };
        };
        let name = p.name.clone();
        match option {
            2 => Flow::Push(Box::new(Rename { pool: name })),
            4 => Flow::Push(Box::new(Typed::new(
                "tools-pool-delete",
                "Confirm Delete of Pool",
                vec![
                    format!("Storage pool {name} is deleted, and its disks are not configured"),
                    "afterwards. A pool that holds a virtual disk or media is not deleted.".into(),
                ],
                name.clone(),
                move |env: &mut Env| asked(env.pools().delete_pool(name.clone())),
            ))),
            5 => Flow::Push(Box::new(Status { pool: name })),
            6 => Flow::Push(Box::new(Choose {
                pool: name,
                purpose: Purpose::Replace,
            })),
            7 => Flow::Push(Box::new(Choose {
                pool: name,
                purpose: Purpose::Spare,
            })),
            8 => Flow::Push(Box::new(Choose {
                pool: name,
                purpose: Purpose::Unspare,
            })),
            9 => Flow::Push(Box::new(Typed::new(
                "tools-pool-repository",
                "Confirm Media Repository",
                vec![
                    format!("Storage pool {name} holds the media library from now on: its"),
                    "optical images and tape volumes. A pool that held it before keeps".into(),
                    "its files, which are no longer the library's.".into(),
                ],
                name.clone(),
                move |env: &mut Env| asked(env.pools().designate(name.clone())),
            ))),
            _ => Flow::Error("Option not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            6 => Flow::Push(Box::new(Create)),
            7 => Flow::Push(Box::new(Import)),
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: POOLS,
            lines: &[
                "The host's storage pools: each a protected set of whole disks",
                "with one file system on it, holding the partitions' virtual",
                "disks and, in the one marked *, the media library.",
                "",
                "Type an option by a pool and press Enter: 5 shows its units and",
                "what it is doing; 6 puts a disk in a failed unit's place; 7 and",
                "8 add and take away a hot spare; 2 renames it; 4 deletes an",
                "empty pool; 9 makes it the media repository.",
                "",
                "F6 makes a pool. F7 imports an image into the media library",
                "from a removable volume or a disc in the host's drive.",
            ],
        }
    }
}

/// What a confirmed request does.
type Action = Box<dyn FnMut(&mut Env) -> Result<(), String>>;

/// A destructive request, confirmed by typing a word.
struct Typed {
    name: &'static str,
    title: &'static str,
    lines: Vec<String>,
    word: String,
    action: Action,
}

impl Typed {
    fn new(
        name: &'static str,
        title: &'static str,
        lines: Vec<String>,
        word: String,
        action: impl FnMut(&mut Env) -> Result<(), String> + 'static,
    ) -> Self {
        Self {
            name,
            title,
            lines,
            word,
            action: Box::new(action),
        }
    }
}

impl Panel<Env, Outcome> for Typed {
    fn name(&self) -> &'static str {
        self.name
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            self.title,
            &env.system(),
            &[FormLine {
                label: "Type it to confirm",
                len: 32,
                value: String::new(),
                hidden: false,
            }],
            "F1=Help   F12=Cancel",
        );
        let mut row = ui::BODY_ROW + 2;
        for line in &self.lines {
            kit::put(&mut s, row, line);
            row += 1;
        }
        kit::put(&mut s, row + 1, &format!("To go on, type: {}", self.word));
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let typed = fields.first().map_or("", |f| f.trim());
        if typed != self.word {
            return Flow::Error(format!("Type {} to confirm, or F12 to go back.", self.word));
        }
        match (self.action)(env) {
            Ok(()) => Flow::Reset(Box::new(Pools)),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm",
            lines: &[
                "What is shown is done when the word shown is typed in the field",
                "and Enter is pressed. A disk erased cannot be had back.",
                "",
                "F12 goes back and nothing is done.",
            ],
        }
    }
}

/// `Create storage pool`: its name and protection.
struct Create;

impl Panel<Env, Outcome> for Create {
    fn name(&self) -> &'static str {
        "tools-pool-create"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Create Storage Pool",
            &env.system(),
            &[
                FormLine {
                    label: "Pool name",
                    len: 10,
                    value: String::new(),
                    hidden: false,
                },
                FormLine {
                    label: "Protection",
                    len: 8,
                    value: "mirrored".into(),
                    hidden: false,
                },
            ],
            "F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 3,
            "Protection: none (1 disk), mirrored (2), raid5 (3 or more),",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 4,
            "raid6 (4 or more). The disks are chosen next.",
        );
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut Env) -> Step {
        let name = fields.first().map_or("", |f| f.trim()).to_uppercase();
        let protection = fields.get(1).map_or("", |f| f.trim()).to_lowercase();
        if !pool::label::valid_name(&name) {
            return Flow::Error(
                "A pool's name is 1 to 10 of A to Z, 0 to 9 and _, a letter first.".into(),
            );
        }
        let Some(protection) = pool::label::Protection::from_word(&protection) else {
            return Flow::Error("Protection is none, mirrored, raid5 or raid6.".into());
        };
        Flow::Push(Box::new(Pick { name, protection }))
    }

    fn help(&self) -> Help {
        Help {
            title: "Create Storage Pool",
            lines: &[
                "A pool's name is how its virtual disks are named: POOL/DISK.",
                "",
                "Protection: none keeps one copy on one disk; mirrored keeps two",
                "copies on two disks; raid5 spreads the data with one disk's",
                "worth of parity over three or more, and holds when one fails;",
                "raid6 with two disks' worth over four or more, and holds when",
                "two fail. A pool's size is fixed when it is made.",
            ],
        }
    }
}

/// The disks of a pool to be made: 1 a member, 2 a hot spare.
struct Pick {
    name: String,
    protection: pool::label::Protection,
}

/// The disks a pool may be made of: the pool service's candidates.
fn candidates(env: &mut Env) -> Vec<Disk> {
    env.pools().candidates().unwrap_or_default()
}

impl Panel<Env, Outcome> for Pick {
    fn name(&self) -> &'static str {
        "tools-pool-disks"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let rows: Vec<String> = candidates(env).iter().map(disk_row).collect();
        let mut s = ui::list(
            "Create Storage Pool",
            &env.system(),
            &format!(
                "Pool {}, {}: type 1 by each member, 2 by each hot spare.",
                self.name,
                self.protection.word()
            ),
            "Opt Location             Serial                Size  What it is",
            &rows,
            "F1=Help   F3=Exit   F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(No disk is free)");
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let all = candidates(env);
        let mut members = Vec::new();
        let mut spares = Vec::new();
        for (d, f) in all.iter().zip(fields) {
            match kit::number(Some(f)) {
                Some(1) => members.push(d.clone()),
                Some(2) => spares.push(d.clone()),
                None => {}
                Some(_) => return Flow::Error("Type 1 or 2.".into()),
            }
        }
        if members.is_empty() {
            return Flow::Error("Type 1 by each disk of the pool.".into());
        }
        let names = |list: &[Disk]| list.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
        let foreign: Vec<Disk> = members
            .iter()
            .chain(&spares)
            .filter(|d| d.foreign)
            .cloned()
            .collect();
        let units = members.len();
        let (m, s) = (names(&members), names(&spares));
        let capacity = env
            .pools()
            .disks()
            .ok()
            .map(|_| plan_capacity(self.protection, &members, &spares));
        let mut lines = vec![
            format!(
                "Pool {}, {} protection: {units} unit(s), {} hot spare(s).",
                self.name,
                self.protection.word(),
                spares.len()
            ),
            match capacity.flatten() {
                Some(bytes) => format!("Its capacity: {}.", kit::size(bytes)),
                None => "The disks chosen do not make a pool: the next step says why.".into(),
            },
            "Everything on these disks is erased.".into(),
        ];
        if !foreign.is_empty() {
            lines.push(String::new());
            lines.push("These hold what is not this host's, and are erased too:".into());
            for d in &foreign {
                lines.push(format!("  {} {}", d.location, kit::fit(&d.what, 50)));
            }
        }
        let (name, protection) = (self.name.clone(), self.protection.word().to_string());
        let confirm = names(&foreign);
        Flow::Push(Box::new(Typed::new(
            "tools-pool-confirm",
            "Confirm Create of Pool",
            lines,
            self.name.clone(),
            move |env: &mut Env| {
                asked(env.pools().create_pool(
                    name.clone(),
                    protection.clone(),
                    m.clone(),
                    s.clone(),
                    confirm.clone(),
                ))
                .map(|_| ())
            },
        )))
    }

    fn help(&self) -> Help {
        Help {
            title: "Choose the disks",
            lines: &[
                "The disks this host has nothing on, and those that hold what is",
                "not this host's. Type 1 by each disk of the pool and 2 by each",
                "hot spare. A hot spare takes a failed unit's place at once and",
                "is rebuilt; a pool with no protection has none.",
                "",
                "The next screen shows the pool's capacity and asks for its name",
                "to be typed; nothing is written before that.",
            ],
        }
    }
}

/// The capacity a pool of `members` would have; `None` when they do not
/// make one.
fn plan_capacity(
    protection: pool::label::Protection,
    members: &[Disk],
    spares: &[Disk],
) -> Option<u64> {
    let candidate = |d: &Disk| pool::plan::Candidate {
        disk: d.name.clone(),
        bytes: d.bytes,
        logical_block: d.logical_block,
        taken: None,
    };
    let m: Vec<_> = members.iter().map(candidate).collect();
    let s: Vec<_> = spares.iter().map(candidate).collect();
    pool::plan::plan(&release::POOL_DISK, protection, &m, &s)
        .ok()
        .map(|p| p.capacity())
}

/// A pool's units and work.
struct Status {
    pool: String,
}

impl Panel<Env, Outcome> for Status {
    fn name(&self) -> &'static str {
        "tools-pool-status"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Some(p) = pool(env, &self.pool) else {
            return no_service("Display Storage Pool", env);
        };
        let mut text = vec![
            format!(
                "{} {}, {} protection, {}",
                kit::dotted("Pool", 10),
                p.name,
                p.protection,
                p.state
            ),
            format!(
                "{} {}, {} allocated, {} provisioned",
                kit::dotted("Size", 10),
                kit::size(p.bytes),
                kit::size(p.allocated),
                kit::size(p.provisioned)
            ),
            String::new(),
            "Unit  Location                Serial                  Size  State".into(),
        ];
        for u in &p.units {
            text.push(format!(
                "{:>4}  {} {} {:>9}  {}",
                u.number,
                kit::fit(&u.location, 23),
                kit::fit(&u.serial, 18),
                if u.bytes == 0 {
                    String::new()
                } else {
                    kit::size(u.bytes)
                },
                u.state
            ));
        }
        text.push(String::new());
        if !p.failure.is_empty() {
            text.push(kit::fit(&format!("Failed: {}", p.failure), 76));
        }
        let bar = if p.work.is_empty() {
            None
        } else {
            let unit = if p.work_unit == 0 {
                "The pool".to_string()
            } else {
                format!("Unit {}", p.work_unit)
            };
            text.push(format!("{unit} is {}.", p.work));
            Some(u8::try_from(p.percent.min(100)).unwrap_or(100))
        };
        kit::enterable(ui::text(
            "Display Storage Pool",
            &env.system(),
            &text,
            bar,
            "Enter=Continue   F1=Help   F12=Cancel",
        ))
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Storage Pool",
            lines: &[
                "A pool's units by location code, serial number and size, each",
                "with its state: Active, Rebuilding, Resuming, Failed, Missing,",
                "or Hot spare. The pool is Active, Degraded (a unit failed and it",
                "runs), Rebuilding, Resynchronizing or Failed (more units lost",
                "than its protection covers).",
                "",
                "A rebuild and a resynchronization give way to the partitions'",
                "own transfers; the bar is how far it has come.",
            ],
        }
    }

    fn live(&self) -> bool {
        true
    }
}

/// Why a disk is to be chosen for a pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    /// In the place of a failed unit.
    Replace,
    /// As a hot spare.
    Spare,
    /// A hot spare to take away.
    Unspare,
}

/// Choose a disk for a pool.
struct Choose {
    pool: String,
    purpose: Purpose,
}

impl Choose {
    fn disks(&self, env: &mut Env) -> Vec<Disk> {
        match self.purpose {
            Purpose::Unspare => disks(env)
                .into_iter()
                .filter(|d| d.pool == self.pool && d.what.starts_with("Hot spare"))
                .collect(),
            _ => candidates(env),
        }
    }
}

impl Panel<Env, Outcome> for Choose {
    fn name(&self) -> &'static str {
        match self.purpose {
            Purpose::Replace => "tools-pool-replace",
            Purpose::Spare => "tools-pool-spare",
            Purpose::Unspare => "tools-pool-unspare",
        }
    }

    fn render(&self, env: &mut Env) -> Screen {
        let rows: Vec<String> = self.disks(env).iter().map(disk_row).collect();
        let prompt = match self.purpose {
            Purpose::Replace => format!(
                "Type 1 by the disk to put in place of {}'s failed unit.",
                self.pool
            ),
            Purpose::Spare => format!("Type 1 by the disk to make a hot spare of {}.", self.pool),
            Purpose::Unspare => format!("Type 1 by the hot spare to take from {}.", self.pool),
        };
        let mut s = ui::list(
            POOLS,
            &env.system(),
            &prompt,
            "Opt Location             Serial                Size  What it is",
            &rows,
            "F1=Help   F3=Exit   F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(No disk to choose)");
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let all = self.disks(env);
        let chosen: Vec<&Disk> = all
            .iter()
            .zip(fields)
            .filter(|(_, f)| !f.trim().is_empty())
            .map(|(d, _)| d)
            .collect();
        let [disk] = chosen.as_slice() else {
            return Flow::Error("Type 1 by one disk.".into());
        };
        let (pool, name, foreign) = (self.pool.clone(), disk.name.clone(), disk.foreign);
        let shown = format!(
            "  {} serial {} {}",
            disk.location,
            disk.serial,
            kit::size(disk.bytes)
        );
        match self.purpose {
            Purpose::Unspare => Flow::Push(Box::new(Typed::new(
                "tools-pool-unspare-confirm",
                "Confirm Remove of Hot Spare",
                vec![
                    format!("This hot spare is taken from {pool}, and is not configured"),
                    "afterwards:".into(),
                    shown,
                ],
                name.clone(),
                move |env: &mut Env| asked(env.pools().remove_spare(pool.clone(), name.clone())),
            ))),
            purpose => {
                let unit = pool_failed_unit(env, &pool);
                let mut lines = vec!["Everything on this disk is erased:".to_string(), shown];
                if foreign {
                    lines.push(format!("It holds what is not this host's: {}.", disk.what));
                }
                lines.push(match purpose {
                    Purpose::Replace => format!(
                        "It takes the place of unit {} and is rebuilt while the host runs.",
                        unit.unwrap_or(0)
                    ),
                    _ => "It waits as a hot spare and takes a failed unit's place.".into(),
                });
                Flow::Push(Box::new(Typed::new(
                    "tools-pool-disk-confirm",
                    "Confirm Erase of Disk",
                    lines,
                    name.clone(),
                    move |env: &mut Env| match purpose {
                        Purpose::Replace => {
                            let Some(unit) = unit else {
                                return Err("No unit of the pool is failed or missing.".into());
                            };
                            asked(env.pools().replace_unit(
                                pool.clone(),
                                unit,
                                name.clone(),
                                foreign,
                            ))
                        }
                        _ => asked(env.pools().add_spare(pool.clone(), name.clone(), foreign)),
                    },
                )))
            }
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Choose a disk",
            lines: &[
                "The disks this host has nothing on, and those that hold what is",
                "not this host's; or, to take one away, the pool's hot spares.",
                "Type 1 by one and press Enter. A replacement or a hot spare is",
                "at least as large as the pool's units.",
                "",
                "The next screen asks for the disk's name to be typed; nothing is",
                "written before that.",
            ],
        }
    }
}

/// The unit of `pool` a replacement takes the place of.
fn pool_failed_unit(env: &mut Env, pool_name: &str) -> Option<u32> {
    pool(env, pool_name)?
        .units
        .iter()
        .find(|u| matches!(u.state.as_str(), "Failed" | "Missing"))
        .map(|u| u.number)
}

/// Rename a pool.
struct Rename {
    pool: String,
}

impl Panel<Env, Outcome> for Rename {
    fn name(&self) -> &'static str {
        "tools-pool-rename"
    }

    fn render(&self, env: &mut Env) -> Screen {
        kit::form(
            "Rename Storage Pool",
            &env.system(),
            &[FormLine {
                label: "New name",
                len: 10,
                value: self.pool.clone(),
                hidden: false,
            }],
            "F1=Help   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let to = fields.first().map_or("", |f| f.trim()).to_uppercase();
        match asked(env.pools().rename_pool(self.pool.clone(), to)) {
            Ok(()) => Flow::Reset(Box::new(Pools)),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Rename Storage Pool",
            lines: &[
                "A pool is renamed while no partition uses it: its virtual disks",
                "are named by the pool's name, POOL/DISK, in the profiles that",
                "use them, which are to be changed with it.",
            ],
        }
    }
}

/// Import an image into the media library from a device of the host.
struct Import;

impl Panel<Env, Outcome> for Import {
    fn name(&self) -> &'static str {
        "tools-media-import"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Import Media Image",
            &env.system(),
            &[
                FormLine {
                    label: "Device",
                    len: 10,
                    value: "sr0".into(),
                    hidden: false,
                },
                FormLine {
                    label: "File on the device",
                    len: 40,
                    value: String::new(),
                    hidden: false,
                },
                FormLine {
                    label: "Image name",
                    len: 32,
                    value: String::new(),
                    hidden: false,
                },
            ],
            "F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 4,
            "Leave the file empty to import the whole disc in an optical drive.",
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let field = |i: usize| fields.get(i).map_or("", |f| f.trim()).to_string();
        let (device, path, name) = (field(0), field(1), field(2));
        match asked(env.pools().import_image(device, path, name)) {
            Ok(()) => Flow::Unwind(
                1,
                Some((
                    "The import started; the history says when it ends.".into(),
                    false,
                )),
            ),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Import Media Image",
            lines: &[
                "Brings an optical image into the media library from the host's",
                "own devices, for a host QS/MC does not manage yet.",
                "",
                "Device: the kernel's name of an optical drive (sr0) or of a",
                "removable volume (sdb1). File: the image's path on that volume;",
                "empty for the whole of the disc in an optical drive. Image name:",
                "its name in the library.",
                "",
                "The import runs while the host does. A file that is not an",
                "ISO 9660 image is refused and nothing of it is kept.",
            ],
        }
    }
}

/// The virtual disks, to be read.
pub struct Vdisks;

impl Panel<Env, Outcome> for Vdisks {
    fn name(&self) -> &'static str {
        "tools-vdisks"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let list = env.pools().vdisks(String::new()).unwrap_or_default();
        let mut text = vec![
            MANAGED.to_string(),
            String::new(),
            "Pool       Disk                       Size  Allocated Kind  Held by".to_string(),
        ];
        if list.is_empty() {
            text.push("(None)".into());
        }
        for d in list.iter().take(kit::BODY_ROWS - 3) {
            let held = if d.holder.partition == 0 {
                String::new()
            } else {
                format!("{} slot {}", d.holder.partition, d.holder.slot)
            };
            text.push(format!(
                "{} {} {:>9} {:>9}  {} {held}",
                kit::fit(&d.pool, 10),
                kit::fit(&d.name, 20),
                kit::size(d.bytes),
                kit::size(d.allocated),
                if d.thick { "thick" } else { "thin " }
            ));
        }
        kit::page(
            "Display Virtual Disks",
            &env.system(),
            &text,
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Virtual Disks",
            lines: &[
                "Every virtual disk of every pool: its size, what the pool has",
                "given it (a thin disk takes space as it is written), and the",
                "partition and slot that hold it.",
                "",
                "They are made, grown and deleted from QS/MC.",
            ],
        }
    }
}

/// The media library, to be read.
pub struct Media;

impl Panel<Env, Outcome> for Media {
    fn name(&self) -> &'static str {
        "tools-media"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let images = env.pools().images().unwrap_or_default();
        let volumes = env.pools().volumes().unwrap_or_default();
        let mut text = vec![
            MANAGED.to_string(),
            String::new(),
            "Optical image                    Volume ID          Size  Loaded in".to_string(),
        ];
        if images.is_empty() {
            text.push("(None)".into());
        }
        for i in images.iter().take(5) {
            let held = if i.holder.partition == 0 {
                String::new()
            } else {
                format!("{} slot {}", i.holder.partition, i.holder.slot)
            };
            text.push(format!(
                "{} {} {:>9}  {held}",
                kit::fit(&i.name, 32),
                kit::fit(&i.label, 15),
                kit::size(i.bytes)
            ));
        }
        text.push(String::new());
        text.push(
            "Tape volume                      ID       Capacity      Used  Mounted in".into(),
        );
        if volumes.is_empty() {
            text.push("(None)".into());
        }
        for v in volumes.iter().take(5) {
            let held = if v.holder.partition == 0 {
                String::new()
            } else {
                format!("{} slot {}", v.holder.partition, v.holder.slot)
            };
            text.push(format!(
                "{} {} {:>9} {:>9}  {held}",
                kit::fit(&v.name, 32),
                kit::fit(&v.identifier, 6),
                kit::size(v.capacity_mib << 20),
                kit::size(v.used)
            ));
        }
        kit::page(
            "Display Media Library",
            &env.system(),
            &text,
            "F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Media Library",
            lines: &[
                "The optical images and tape volumes of the media library, in the",
                "pool that is the media repository, each with the partition and",
                "slot it is loaded in.",
                "",
                "They are uploaded, made, loaded and deleted from QS/MC. An image",
                "can be imported here from the host's own devices: option 9 of",
                "Work with storage pools makes a pool the repository, and F7",
                "there imports.",
            ],
        }
    }
}
