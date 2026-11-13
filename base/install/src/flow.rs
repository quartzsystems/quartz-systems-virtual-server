// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The installer's screens, in order: welcome, the load-source disks,
//! the confirmation that erases them, the host name and network, the
//! time, the service tools user, the summary, the progress and the
//! end. Each field is checked when Enter is pressed on its screen and
//! the reason is on the message line. The first write is the first
//! step of the progress display.

use console::{Attr, Screen};
use panels::forms::{NetworkEntry, TimeChoice, TimeEntry, UserEntry};
use panels::ui;
use panels::{Flow, Help, Named, Panel, PanelFlow, Runner, kit};
use state::Network;

use crate::{Answers, DiskChoice, Outcome, Target};

/// The word that confirms the erasure.
pub const ERASE: &str = "ERASE";

/// What the installer's panels ask: the product, and what has been
/// answered so far.
pub struct Env<'a> {
    /// The product.
    pub target: &'a mut dyn Target,
    disks: Vec<DiskChoice>,
    host: String,
    network: Network,
    net_entry: Option<NetworkEntry>,
    time: Option<TimeChoice>,
    time_entry: TimeEntry,
    user_id: String,
    password_hash: String,
}

impl<'a> Env<'a> {
    /// Nothing answered yet.
    pub fn new(target: &'a mut dyn Target) -> Self {
        let host = target.default_host();
        Self {
            target,
            disks: Vec::new(),
            host,
            network: Network::default(),
            net_entry: None,
            time: None,
            time_entry: TimeEntry::default(),
            user_id: String::new(),
            password_hash: String::new(),
        }
    }

    fn title(&self) -> String {
        format!("Install {}", self.target.product())
    }

    fn answers(&self) -> Answers {
        Answers {
            disks: self.disks.clone(),
            host: self.host.clone(),
            network: self.network.clone(),
            time: self.time.clone().unwrap_or(TimeChoice {
                zone: "UTC".into(),
                servers: Vec::new(),
                clock: None,
            }),
            user_id: self.user_id.clone(),
            password_hash: self.password_hash.clone(),
        }
    }
}

impl Named for Env<'_> {
    fn system(&self) -> String {
        self.host.clone()
    }
}

type Step<'a> = PanelFlow<Env<'a>, Outcome>;

/// The installer an operator answers.
pub fn attended<'a>() -> Runner<Env<'a>, Outcome> {
    Runner::new(Box::new(Welcome))
}

/// The installer that was given its answers: the progress display at
/// once, and a restart by itself.
pub fn unattended<'a>(answers: Answers) -> Runner<Env<'a>, Outcome> {
    Runner::new(Box::new(Progress::new(answers, false)))
}

/// The installer that found answers it cannot use.
pub fn stopped<'a>(reason: String) -> Runner<Env<'a>, Outcome> {
    Runner::new(Box::new(Stopped {
        name: "install-answers",
        heading: "The answers of an unattended installation were found and cannot be used:",
        reason,
        keys: "Enter=Install by hand   F3=Exit",
        attended: true,
    }))
}

/// The installer on media it does not trust.
pub fn refused<'a>(reason: String) -> Runner<Env<'a>, Outcome> {
    Runner::new(Box::new(Stopped {
        name: "install-media",
        heading: "This installation media is refused. Nothing was written.",
        reason,
        keys: "F3=Exit",
        attended: false,
    }))
}

/// A reason as a screen words it: a capital first, unless it starts
/// with a file's name, and a full stop last.
fn sentence(reason: &str) -> String {
    let named = reason.split(' ').next().is_some_and(|w| w.contains('.'));
    let mut chars = reason.chars();
    let first = match chars.next() {
        Some(c) if !named => c.to_uppercase().collect::<String>(),
        Some(c) => c.to_string(),
        None => String::new(),
    };
    let rest = chars.as_str();
    let stop = if rest.ends_with('.') { "" } else { "." };
    format!("{first}{rest}{stop}")
}

/// Break `text` into lines of at most `width` characters, at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split_whitespace() {
        let last = lines.last_mut().expect("never empty");
        if !last.is_empty() && last.len() + 1 + word.len() > width {
            lines.push(word.to_string());
        } else {
            if !last.is_empty() {
                last.push(' ');
            }
            last.push_str(word);
        }
    }
    lines
}

/// A screen that says why the installer does not go on.
struct Stopped {
    name: &'static str,
    heading: &'static str,
    reason: String,
    keys: &'static str,
    attended: bool,
}

impl<'a> Panel<Env<'a>, Outcome> for Stopped {
    fn name(&self) -> &'static str {
        self.name
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let mut lines = vec![self.heading.to_string(), String::new()];
        lines.extend(wrap(&sentence(&self.reason), 74));
        kit::page(&env.title(), &env.system(), &lines, self.keys)
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        if self.attended {
            Flow::Reset(Box::new(Welcome))
        } else {
            Flow::Stay
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        match key {
            3 | 12 => Flow::Exit(Outcome::Left),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Installation stopped",
            lines: &[
                "The installer found something it cannot go on with and says",
                "what. Nothing was written to any disk.",
                "",
                "F3 leaves the installer and restarts the machine. Correct what",
                "is named, then start the installation again.",
            ],
        }
    }
}

struct Welcome;

impl<'a> Panel<Env<'a>, Outcome> for Welcome {
    fn name(&self) -> &'static str {
        "install-welcome"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let mut lines = vec![env.target.banner(), String::new()];
        lines.extend(kit::lines(env.target.what()));
        lines.push(String::new());
        lines.push("Nothing is written until you have seen the summary and pressed F10.".into());
        lines.push(String::new());
        lines.push("Press Enter to continue, or F3 to leave without installing.".into());
        kit::page(
            &env.title(),
            &env.system(),
            &lines,
            "Enter=Continue   F1=Help   F3=Exit",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        Flow::Push(Box::new(Disks))
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        match key {
            3 | 12 => Flow::Exit(Outcome::Left),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Welcome",
            lines: &[
                "This installs the product on the disks of this machine. The",
                "disks you choose are erased; no other disk is touched.",
                "",
                "The screens that follow ask for the load-source disks, the host",
                "name and management network, the time, and a service tools user.",
                "A summary shows what you chose. F10 on it starts the installation;",
                "until then F12 goes back a screen and F3 leaves.",
                "",
                "F1 shows help on every screen.",
            ],
        }
    }
}

const DISK_HEADINGS: &str =
    "Opt Location                Model               Serial                  Size";

fn disk_row(disk: &DiskChoice, least: u64) -> String {
    format!(
        "{} {} {} {:>9}{}",
        kit::fit(&disk.location, 23),
        kit::fit(&disk.model, 19),
        kit::fit(&disk.serial, 18),
        kit::size(disk.bytes),
        if disk.bytes < least { " *" } else { "" }
    )
}

struct Disks;

impl<'a> Panel<Env<'a>, Outcome> for Disks {
    fn name(&self) -> &'static str {
        "install-disks"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let least = env.target.min_disk_bytes();
        let disks = env.target.disks();
        let rows: Vec<String> = disks.iter().map(|d| disk_row(d, least)).collect();
        let mut s = ui::list(
            &env.title(),
            &env.system(),
            "Type 1 by the load-source disk, or by two disks for mirrored protection.",
            DISK_HEADINGS,
            &rows,
            "Enter=Continue   F1=Help   F3=Exit   F12=Cancel",
        );
        let foot = ui::BODY_ROW + 4 + rows.len();
        if disks.is_empty() {
            kit::put(&mut s, ui::BODY_ROW + 3, "(No disk was found)");
        }
        if disks.iter().any(|d| d.bytes < least) && foot < ui::KEYS_ROW - 1 {
            kit::put(
                &mut s,
                foot,
                &format!(
                    "* Smaller than a load source may be ({}); cannot be chosen.",
                    kit::size(least)
                ),
            );
        }
        if env.disks.len() == 2 && foot + 1 < ui::KEYS_ROW - 1 {
            kit::put(
                &mut s,
                foot + 1,
                "Two disks are chosen: mirrored protection.",
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env<'a>) -> Step<'a> {
        let least = env.target.min_disk_bytes();
        let disks = env.target.disks();
        let mut chosen = Vec::new();
        for (disk, field) in disks.iter().zip(fields) {
            match field.trim() {
                "" => {}
                "1" if disk.bytes < least => {
                    return Flow::Error(format!(
                        "Disk at {} is too small to be a load source.",
                        disk.location
                    ));
                }
                "1" => chosen.push(disk.clone()),
                _ => return Flow::Error("Option must be 1, or blank.".into()),
            }
        }
        match chosen.len() {
            0 => Flow::Error("Choose one disk, or two for mirrored protection.".into()),
            1 | 2 => {
                env.disks = chosen;
                Flow::Push(Box::new(Erase))
            }
            n => Flow::Error(format!(
                "{n} disks are chosen. Choose one, or two for mirrored protection."
            )),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Select load-source disks",
            lines: &[
                "The load source is the disk the host starts from and keeps its",
                "own state on. Partitions' storage goes on other disks, later.",
                "",
                "Type 1 by one disk for an unprotected load source. Type 1 by two",
                "disks for mirrored protection: the host then starts from either",
                "disk alone and runs on with one of them failed. Two disks of",
                "different size are mirrored to the size of the smaller.",
                "",
                "Each disk is shown by its location code, which names the slot or",
                "the bus position it is in, with its model, serial number and size.",
                "A disk marked * is too small and cannot be chosen.",
            ],
        }
    }
}

/// Where the confirmation is typed.
const ERASE_ROW: usize = 16;

struct Erase;

impl<'a> Panel<Env<'a>, Outcome> for Erase {
    fn name(&self) -> &'static str {
        "install-erase"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let mut lines = vec![
            "Everything on these disks will be erased:".to_string(),
            String::new(),
        ];
        for disk in &env.disks {
            lines.push(format!(
                "  {}  {}  serial {}  {}",
                disk.location,
                disk.model,
                disk.serial,
                kit::size(disk.bytes)
            ));
        }
        lines.push(String::new());
        lines.push(
            if env.disks.len() == 2 {
                "They become a mirrored load source."
            } else {
                "It becomes an unprotected load source."
            }
            .into(),
        );
        lines.push(String::new());
        lines.push("Nothing is erased yet. The disks are written after the summary.".into());
        let mut s = ui::text(
            &env.title(),
            &env.system(),
            &lines,
            None,
            "Enter=Continue   F1=Help   F3=Exit   F12=Cancel",
        );
        let label = kit::dotted(&format!("Type {ERASE} to confirm"), 29);
        s.put(ERASE_ROW, ui::BODY_COL, &label, Attr::GREEN);
        s.field(
            ERASE_ROW,
            ui::BODY_COL + label.len() + 1,
            ERASE.len(),
            false,
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env<'a>) -> Step<'a> {
        if fields.first().map(|f| f.trim()) != Some(ERASE) {
            return Flow::Error(format!(
                "Type {ERASE} in capital letters to confirm, or press F12 to go back."
            ));
        }
        let links = env.target.links();
        let entry = env
            .net_entry
            .clone()
            .unwrap_or_else(|| NetworkEntry::of(&env.host, &env.network, &links));
        Flow::Push(Box::new(Net { entry }))
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm erase",
            lines: &[
                "The disks named are the ones you chose. Installing erases them",
                "entirely: every partition and everything in it.",
                "",
                "To go on, type the word shown, in capital letters, and press",
                "Enter. F12 goes back to choose other disks.",
                "",
                "This screen writes nothing. The disks are written only after you",
                "press F10 on the summary.",
            ],
        }
    }
}

const FORM_KEYS: &str = "Enter=Continue   F1=Help   F3=Exit   F12=Cancel";

struct Net {
    entry: NetworkEntry,
}

impl<'a> Panel<Env<'a>, Outcome> for Net {
    fn name(&self) -> &'static str {
        "install-network"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let links = env.target.links();
        self.entry
            .screen(&env.title(), &env.system(), &links, FORM_KEYS)
    }

    fn enter(&mut self, fields: &[String], env: &mut Env<'a>) -> Step<'a> {
        self.entry = NetworkEntry::take(fields);
        let links = env.target.links();
        match self.entry.check(&links) {
            Ok((host, network)) => {
                env.host = host;
                env.network = network;
                env.net_entry = Some(self.entry.clone());
                Flow::Push(Box::new(Time {
                    entry: env.time_entry.clone(),
                }))
            }
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Host name and management network",
            lines: &[
                "Host name: what the host calls itself. Letters, digits, hyphens.",
                "",
                "Interface: the number of an interface in the list. The list shows",
                "each by location code and hardware address, and whether its link",
                "has carrier now. Blank takes the first that has carrier at IPL.",
                "",
                "Address: 1 takes the address from a DHCP server. 2 uses the IP",
                "address, prefix length (24 for 255.255.255.0) and gateway typed",
                "here; the gateway may be left blank.",
                "",
                "VLAN ID: 1 to 4094 when the management network is a tagged VLAN.",
                "DNS servers: addresses, separated by commas.",
            ],
        }
    }
}

struct Time {
    entry: TimeEntry,
}

impl<'a> Panel<Env<'a>, Outcome> for Time {
    fn name(&self) -> &'static str {
        "install-time"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let now = env.target.now();
        self.entry
            .screen(&env.title(), &env.system(), now, FORM_KEYS)
    }

    fn enter(&mut self, fields: &[String], env: &mut Env<'a>) -> Step<'a> {
        self.entry = TimeEntry::take(fields);
        match self.entry.check() {
            Ok(choice) => {
                env.time = Some(choice);
                env.time_entry = self.entry.clone();
                Flow::Push(Box::new(User))
            }
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Time",
            lines: &[
                "The host keeps its clock in UTC. The UTC offset says how dates",
                "and times are displayed on this console: +01:00 is one hour east",
                "of UTC, -05:00 five hours west. Blank displays UTC.",
                "",
                "Time servers: when given, the host keeps its clock in step with",
                "them once the management network is up.",
                "",
                "Without time servers the clock is the machine's own. The screen",
                "shows what it says now. To correct it, type the date and the",
                "time as they are now, in the offset you gave. Blank leaves it.",
            ],
        }
    }
}

struct User;

impl<'a> Panel<Env<'a>, Outcome> for User {
    fn name(&self) -> &'static str {
        "install-user"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        UserEntry::screen(&env.user_id, &env.title(), &env.system(), FORM_KEYS)
    }

    fn enter(&mut self, fields: &[String], env: &mut Env<'a>) -> Step<'a> {
        let entry = UserEntry::take(fields);
        env.user_id.clone_from(&entry.id);
        let (id, password) = match entry.check() {
            Ok(checked) => checked,
            Err(e) => return Flow::Error(e),
        };
        match users::hash(&password) {
            Ok(hash) => {
                env.user_id = id;
                env.password_hash = hash;
                Flow::Push(Box::new(Summary))
            }
            Err(e) => Flow::Error(format!("The password could not be kept: {e}.")),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Service tools user",
            lines: &[
                "Service Tools is the console on this machine's display and",
                "serial port. This is the first user who can sign on to it.",
                "More are added there.",
                "",
                "Type a user ID, and the password twice. The rules are on the",
                "screen. What you type in the password fields is not shown, and",
                "the password is kept nowhere: only a salted hash of it is.",
                "",
                "Keep the password. Without a user who can sign on, the host is",
                "configured only by installing it again.",
            ],
        }
    }
}

fn summary(answers: &Answers) -> Vec<String> {
    let mut lines = Vec::new();
    let row = |label: &str, value: &str| format!("{} {value}", kit::dotted(label, 23));
    for (i, disk) in answers.disks.iter().enumerate() {
        lines.push(row(
            if i == 0 {
                "Load-source disk"
            } else {
                "Mirrored with"
            },
            &format!(
                "{}  serial {}  {}",
                disk.location,
                disk.serial,
                kit::size(disk.bytes)
            ),
        ));
    }
    lines.push(row(
        "Protection",
        if answers.mirrored() {
            "Mirrored"
        } else {
            "None"
        },
    ));
    lines.push(row("Host name", &answers.host));
    let n = &answers.network;
    lines.push(row("Interface", &n.device.to_string()));
    lines.push(row(
        "Address",
        &match n.address {
            Some(address) => format!("{address} (static)"),
            None => "from DHCP".into(),
        },
    ));
    if let Some(gateway) = n.gateway {
        lines.push(row("Gateway", &gateway.to_string()));
    }
    if let Some(vlan) = n.vlan {
        lines.push(row("VLAN ID", &vlan.to_string()));
    }
    if !n.dns.is_empty() {
        let dns: Vec<String> = n.dns.iter().map(ToString::to_string).collect();
        lines.push(row("DNS servers", &dns.join(", ")));
    }
    lines.push(row("Time zone", &answers.time.zone));
    lines.push(row(
        "Time servers",
        &if answers.time.servers.is_empty() {
            "none".to_string()
        } else {
            answers.time.servers.join(", ")
        },
    ));
    if answers.time.clock.is_some() {
        lines.push(row("Clock", "set as typed"));
    }
    lines.push(row("Service tools user", &answers.user_id));
    lines
}

struct Summary;

impl<'a> Panel<Env<'a>, Outcome> for Summary {
    fn name(&self) -> &'static str {
        "install-summary"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let mut lines = summary(&env.answers());
        lines.push(String::new());
        lines.push("F10 erases the disks named and installs. F12 goes back.".into());
        kit::page(
            &env.title(),
            &env.system(),
            &lines,
            "F1=Help   F3=Exit   F10=Install   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        Flow::Message("Press F10 to install, or F12 to go back.".into())
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut Env<'a>) -> Step<'a> {
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            10 => Flow::Reset(Box::new(Progress::new(env.answers(), true))),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Summary",
            lines: &[
                "This is everything you chose. Nothing has been written yet.",
                "",
                "F10 starts the installation. The disks named are erased and",
                "written; this cannot be undone, and from then on there is no",
                "going back.",
                "",
                "F12 goes back a screen at a time to change an answer. F3 goes",
                "back to the welcome.",
            ],
        }
    }
}

/// The progress display. Each tick carries out one step.
struct Progress {
    answers: Answers,
    attended: bool,
    done: usize,
    failed: Option<String>,
}

impl Progress {
    fn new(answers: Answers, attended: bool) -> Self {
        Self {
            answers,
            attended,
            done: 0,
            failed: None,
        }
    }
}

impl<'a> Panel<Env<'a>, Outcome> for Progress {
    fn name(&self) -> &'static str {
        "install-progress"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let steps = env.target.steps();
        let mut lines = vec![
            "Installing. Do not power the machine off.".to_string(),
            String::new(),
        ];
        for (i, step) in steps.iter().enumerate() {
            let mark = match (i.cmp(&self.done), &self.failed) {
                (std::cmp::Ordering::Less, _) => "X",
                (std::cmp::Ordering::Equal, Some(_)) => "!",
                (std::cmp::Ordering::Equal, None) => ">",
                (std::cmp::Ordering::Greater, _) => " ",
            };
            lines.push(format!("  {mark}  {}  {}", step.code, step.name));
        }
        let mut keys = "";
        if let Some(reason) = &self.failed {
            let step = steps.get(self.done).map_or("?", |s| s.name);
            lines.push(String::new());
            lines.push(format!(
                "{} Installation failed at: {step}",
                env.target.failed()
            ));
            lines.extend(wrap(reason, 74).into_iter().take(3));
            keys = "Enter=Start over   F3=Exit";
        }
        let percent = u8::try_from(self.done * 100 / steps.len().max(1)).unwrap_or(100);
        let bar = self.failed.is_none().then_some(percent);
        kit::enterable(ui::text(
            &env.title(),
            &self.answers.host,
            &lines,
            bar,
            keys,
        ))
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        if self.failed.is_some() {
            Flow::Reset(Box::new(Welcome))
        } else {
            Flow::Stay
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        match (key, &self.failed) {
            (3, Some(_)) => Flow::Exit(Outcome::Left),
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Installation progress",
            lines: &[
                "The steps of the installation, each with its reference code.",
                "X marks a step that is done, > the one under way.",
                "",
                "If a step fails it is marked ! and named with the reason, under",
                "the code of a failed installation. The disks are then partly",
                "written. Enter starts the installer over; it writes every disk",
                "from the beginning, so nothing of the failed attempt is used.",
            ],
        }
    }

    fn live(&self) -> bool {
        self.failed.is_none()
    }

    fn tick(&mut self, env: &mut Env<'a>) -> Step<'a> {
        let steps = env.target.steps();
        let Some(step) = steps.get(self.done) else {
            return Flow::Replace(Box::new(Complete {
                attended: self.attended,
            }));
        };
        match env.target.step(self.done, &self.answers) {
            Ok(()) => {
                env.target.log(&format!("{} {}", step.code, step.name));
                self.done += 1;
                // The screen is drawn again with the step marked.
                Flow::Message(String::new())
            }
            Err(reason) => {
                let failed = env.target.failed();
                env.target.log(&format!(
                    "{failed} Installation failed: {}: {reason}",
                    step.name
                ));
                self.failed = Some(reason);
                Flow::Error(format!("Installation failed at: {}.", step.name))
            }
        }
    }
}

struct Complete {
    attended: bool,
}

impl<'a> Panel<Env<'a>, Outcome> for Complete {
    fn name(&self) -> &'static str {
        "install-complete"
    }

    fn render(&self, env: &mut Env<'a>) -> Screen {
        let product = env.target.product();
        let mut lines = vec![format!("{product} is installed."), String::new()];
        if self.attended {
            lines.push("Remove the installation media.".into());
            lines.push(String::new());
            lines.push("Press Enter to restart. The host then starts from its load source.".into());
        } else {
            lines.push("The host restarts now and starts from its load source.".into());
        }
        kit::page(
            &env.title(),
            &env.system(),
            &lines,
            if self.attended { "Enter=Restart" } else { "" },
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        Flow::Exit(Outcome::Installed)
    }

    fn function(&mut self, _key: u8, _fields: &[String], _env: &mut Env<'a>) -> Step<'a> {
        Flow::Stay
    }

    fn help(&self) -> Help {
        Help {
            title: "Installation complete",
            lines: &[
                "The installation is finished and the disks are complete.",
                "",
                "Take the installation media out, or detach the virtual media,",
                "so the machine does not start the installer again. Then press",
                "Enter. The host restarts and comes up from its load source;",
                "sign on to Service Tools with the user you made.",
            ],
        }
    }

    fn live(&self) -> bool {
        !self.attended
    }

    fn tick(&mut self, _env: &mut Env<'a>) -> Step<'a> {
        Flow::Exit(Outcome::Installed)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Step as InstallStep;
    use panels::End;
    use panels::forms::LinkChoice;

    const STEPS: &[InstallStep] = &[
        InstallStep {
            name: "Partitioning the disks",
            code: "C0000020",
        },
        InstallStep {
            name: "Writing state",
            code: "C0000080",
        },
    ];

    /// A machine with three disks and one link; its steps only count.
    #[derive(Default)]
    pub(crate) struct Machine {
        pub steps_run: Vec<usize>,
        pub fail_at: Option<usize>,
        pub seen: Option<Answers>,
        pub log: Vec<String>,
    }

    fn disk(name: &str, location: &str, serial: &str, bytes: u64) -> DiskChoice {
        DiskChoice {
            name: name.into(),
            location: location.into(),
            model: "virtio-blk".into(),
            serial: serial.into(),
            bytes,
        }
    }

    impl Target for Machine {
        fn product(&self) -> &'static str {
            "QSDEMO"
        }
        fn banner(&self) -> String {
            "QSDEMO V0R1M0 test".into()
        }
        fn what(&self) -> &'static [&'static str] {
            &["This installs QSDEMO."]
        }
        fn disks(&mut self) -> Vec<DiskChoice> {
            vec![
                disk("vda", "P0000-B00-D05-F0-U0", "SER-A", 2 << 30),
                disk("vdb", "P0000-B00-D06-F0-U0", "SER-B", 1 << 30),
                disk("vdc", "P0000-B00-D07-F0-U0", "SER-SMALL", 100 << 20),
            ]
        }
        fn min_disk_bytes(&self) -> u64 {
            1 << 30
        }
        fn links(&mut self) -> Vec<LinkChoice> {
            vec![LinkChoice {
                name: "eth0".into(),
                mac: "52:54:00:12:34:56".into(),
                location: "P0000-B00-D04-F0-E0".into(),
                carrier: true,
            }]
        }
        fn default_host(&self) -> String {
            "QSDEMO123456".into()
        }
        fn now(&self) -> i64 {
            1_794_488_700
        }
        fn steps(&self) -> &'static [InstallStep] {
            STEPS
        }
        fn step(&mut self, index: usize, answers: &Answers) -> Result<(), String> {
            self.steps_run.push(index);
            self.seen = Some(answers.clone());
            if self.fail_at == Some(index) {
                return Err("the disk said no".into());
            }
            Ok(())
        }
        fn failed(&self) -> &'static str {
            "A0000090"
        }
        fn log(&mut self, line: &str) {
            self.log.push(line.into());
        }
    }

    type Run<'a> = Runner<Env<'a>, Outcome>;

    fn text<'a>(r: &Run<'a>, env: &mut Env<'a>) -> String {
        r.screen(env).dump()
    }

    fn message<'a>(r: &Run<'a>, env: &mut Env<'a>) -> String {
        text(r, env).lines().nth(23).unwrap().trim().to_string()
    }

    fn enter<'a>(r: &mut Run<'a>, env: &mut Env<'a>, fields: &[&str]) -> Option<End<Outcome>> {
        let fields: Vec<String> = fields.iter().map(ToString::to_string).collect();
        r.enter(&fields, env)
    }

    const NET: [&str; 8] = ["vs-01", "", "1", "", "", "", "", ""];
    const TIME: [&str; 4] = ["+01:00", "", "", ""];
    const USER: [&str; 3] = ["admin", "pass word 1", "pass word 1"];

    /// Through the screens to the summary.
    fn to_summary<'a>(r: &mut Run<'a>, env: &mut Env<'a>, disks: &[&str]) {
        assert_eq!(r.top(), "install-welcome");
        enter(r, env, &[""]);
        enter(r, env, disks);
        assert_eq!(r.top(), "install-erase", "{}", message(r, env));
        enter(r, env, &[ERASE]);
        enter(r, env, &NET);
        assert_eq!(r.top(), "install-time", "{}", message(r, env));
        enter(r, env, &TIME);
        enter(r, env, &USER);
        assert_eq!(r.top(), "install-summary", "{}", message(r, env));
    }

    #[test]
    fn an_attended_installation_goes_through_the_nine_screens() {
        let mut machine = Machine::default();
        let mut env = Env::new(&mut machine);
        let mut r = attended();
        assert!(text(&r, &mut env).contains("Install QSDEMO"));
        assert!(text(&r, &mut env).contains("QSDEMO V0R1M0 test"));
        to_summary(&mut r, &mut env, &["1", "1", ""]);
        let summary = text(&r, &mut env);
        for wanted in [
            "Load-source disk . . . : P0000-B00-D05-F0-U0  serial SER-A  2.1 GB",
            "Mirrored with  . . . . : P0000-B00-D06-F0-U0  serial SER-B  1.0 GB",
            "Protection . . . . . . : Mirrored",
            "Host name  . . . . . . : vs-01",
            "Address  . . . . . . . : from DHCP",
            "Time zone  . . . . . . : UTC+01:00",
            "Service tools user . . : ADMIN",
        ] {
            assert!(summary.contains(wanted), "{wanted}\n{summary}");
        }
        assert!(!summary.contains("pass word"));
        // Nothing was written before F10.
        assert_eq!(env.target.steps().len(), 2);
        assert_eq!(r.function(10, &[], &mut env), None);
        assert_eq!(r.top(), "install-progress");
        assert!(text(&r, &mut env).contains("  >  C0000020  Partitioning the disks"));
        assert_eq!(r.tick(&mut env), None);
        assert!(text(&r, &mut env).contains("  X  C0000020  Partitioning the disks"));
        assert!(text(&r, &mut env).contains("  >  C0000080  Writing state"));
        assert_eq!(r.tick(&mut env), None);
        assert_eq!(r.tick(&mut env), None);
        assert_eq!(r.top(), "install-complete");
        assert!(text(&r, &mut env).contains("Remove the installation media."));
        assert_eq!(
            enter(&mut r, &mut env, &[""]),
            Some(End::Outcome(Outcome::Installed))
        );
        drop(r);
        drop(env);
        assert_eq!(machine.steps_run, [0, 1]);
        let answers = machine.seen.unwrap();
        assert_eq!(answers.disks.len(), 2);
        assert!(users::valid_hash(&answers.password_hash));
        assert_eq!(
            machine.log,
            ["C0000020 Partitioning the disks", "C0000080 Writing state"]
        );
    }

    #[test]
    fn every_screen_checks_what_is_typed_and_writes_nothing() {
        let mut machine = Machine::default();
        let mut env = Env::new(&mut machine);
        let mut r = attended();
        enter(&mut r, &mut env, &[""]);
        let disks = text(&r, &mut env);
        assert!(disks.contains("P0000-B00-D07-F0-U0"));
        assert!(disks.contains("104.8 MB *"));
        for (fields, why) in [
            (["", "", ""], "Choose one disk"),
            (["1", "1", "1"], "too small"),
            (["", "", "1"], "too small"),
            (["x", "", ""], "Option must be 1"),
        ] {
            enter(&mut r, &mut env, &fields);
            assert!(message(&r, &mut env).contains(why), "{fields:?}");
            assert_eq!(r.top(), "install-disks");
        }
        enter(&mut r, &mut env, &["1", "", ""]);
        assert!(text(&r, &mut env).contains("It becomes an unprotected load source."));
        enter(&mut r, &mut env, &["erase"]);
        assert!(message(&r, &mut env).contains("capital letters"));
        enter(&mut r, &mut env, &[ERASE]);
        let mut bad = NET;
        bad[0] = "bad name";
        enter(&mut r, &mut env, &bad);
        assert!(message(&r, &mut env).starts_with("Host name"));
        // What was typed stays in the fields.
        assert!(text(&r, &mut env).contains("bad name"));
        enter(&mut r, &mut env, &NET);
        enter(&mut r, &mut env, &["bad", "", "", ""]);
        assert!(message(&r, &mut env).starts_with("UTC offset"));
        enter(&mut r, &mut env, &TIME);
        enter(&mut r, &mut env, &["admin", "short", "short"]);
        assert!(message(&r, &mut env).contains("at least 8"));
        assert!(!text(&r, &mut env).contains("short"));
        // F12 goes back a screen, with the answers kept.
        r.function(12, &[], &mut env);
        assert_eq!(r.top(), "install-time");
        assert!(text(&r, &mut env).contains("+01:00"));
        drop(r);
        drop(env);
        assert!(machine.steps_run.is_empty());
    }

    #[test]
    fn a_failed_step_is_named_and_the_installer_starts_over() {
        let mut machine = Machine {
            fail_at: Some(1),
            ..Machine::default()
        };
        let mut env = Env::new(&mut machine);
        let mut r = attended();
        to_summary(&mut r, &mut env, &["1", "", ""]);
        r.function(10, &[], &mut env);
        r.tick(&mut env);
        r.tick(&mut env);
        let failed = text(&r, &mut env);
        assert!(failed.contains("  !  C0000080  Writing state"), "{failed}");
        assert!(failed.contains("A0000090 Installation failed at: Writing state"));
        assert!(failed.contains("the disk said no"));
        // It stops there: no further step, no going back into the forms.
        assert_eq!(r.tick(&mut env), None);
        assert_eq!(r.function(12, &[], &mut env), None);
        assert_eq!(r.top(), "install-progress");
        enter(&mut r, &mut env, &[""]);
        assert_eq!(r.top(), "install-welcome");
        assert_eq!(r.depth(), 1);
        drop(r);
        drop(env);
        assert_eq!(machine.steps_run, [0, 1]);
        assert!(machine.log[1].starts_with("A0000090 Installation failed: Writing state"));
    }

    #[test]
    fn an_unattended_installation_shows_only_the_progress_and_ends_by_itself() {
        let mut machine = Machine::default();
        let answers = {
            let mut env = Env::new(&mut machine);
            let mut r = attended();
            to_summary(&mut r, &mut env, &["1", "", ""]);
            env.answers()
        };
        let mut env = Env::new(&mut machine);
        let mut r = unattended(answers);
        assert_eq!(r.top(), "install-progress");
        assert!(r.live());
        r.tick(&mut env);
        r.tick(&mut env);
        assert_eq!(r.tick(&mut env), None);
        assert_eq!(r.top(), "install-complete");
        assert!(text(&r, &mut env).contains("The host restarts now"));
        assert_eq!(r.tick(&mut env), Some(End::Outcome(Outcome::Installed)));
    }

    #[test]
    fn answers_that_cannot_be_used_and_media_that_is_refused_stop_on_a_screen() {
        let mut machine = Machine::default();
        let mut env = Env::new(&mut machine);
        let mut r = stopped("seed.toml: [install]: there is no disk SER-X".into());
        let shown = text(&r, &mut env);
        assert!(shown.contains("cannot be used"));
        assert!(shown.contains("seed.toml: [install]: there is no disk SER-X."));
        assert_eq!(
            sentence("the media is of another release"),
            "The media is of another release."
        );
        enter(&mut r, &mut env, &[""]);
        assert_eq!(r.top(), "install-welcome");

        let mut r = refused("the manifest's signature is by no trusted key".into());
        assert!(text(&r, &mut env).contains("Nothing was written."));
        assert_eq!(enter(&mut r, &mut env, &[""]), None);
        assert_eq!(r.top(), "install-media");
        assert_eq!(
            r.function(3, &[], &mut env),
            Some(End::Outcome(Outcome::Left))
        );
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
    }
}
