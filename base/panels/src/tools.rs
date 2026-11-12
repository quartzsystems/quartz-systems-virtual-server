// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The panels every service console has: sign-on, the management
//! network, service tools user IDs, date and time, the logs, and power.
//! They ask the appliance for everything through [`Host`] and change
//! nothing themselves: the appliance's services own the state.

use console::Screen;
use scrkit::panel::Flow;
use scrkit::ui::{self, FormLine};
use state::{Host as HostFile, Network};

use crate::forms::{LinkChoice, NetworkEntry, TimeChoice, TimeEntry, UserEntry};
use crate::kit;
use crate::panel::{Help, Named, Panel, PanelFlow};

/// One line of a log, as a display shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// When, as displayed.
    pub time: String,
    /// 0 for information, 1 for a warning, 2 for an error.
    pub severity: u8,
    /// What: a reference code and its text, or a source and its message.
    pub text: String,
}

/// How a change of the network that is on trial stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trial {
    /// Being tried; taken back in this many seconds.
    Trying(u32),
    /// Kept and written.
    Kept,
    /// Taken back, and why.
    Restored(String),
    /// Nothing is known of it.
    Unknown,
}

/// What the common panels ask of the appliance.
pub trait Host: Named {
    /// The clock: seconds since the epoch.
    fn now(&self) -> i64;
    /// Check a service tools user ID and password: the ID as kept, or
    /// what to tell the operator. The appliance logs the attempt.
    fn sign_on(&mut self, id: &str, password: &str) -> Result<String, String>;
    /// The service tools user IDs.
    fn user_ids(&mut self) -> Vec<String>;
    /// Add a user.
    fn add_user(&mut self, id: &str, password: &str) -> Result<(), String>;
    /// Give a user a new password.
    fn set_password(&mut self, id: &str, password: &str) -> Result<(), String>;
    /// Remove a user.
    fn remove_user(&mut self, id: &str) -> Result<(), String>;
    /// The Ethernet links.
    fn links(&mut self) -> Vec<LinkChoice>;
    /// The host's settings, its name filled in, and the network's.
    fn settings(&mut self) -> (HostFile, Network);
    /// Change the host name.
    fn set_name(&mut self, name: &str) -> Result<(), String>;
    /// Put a network configuration on trial.
    fn apply_network(&mut self, network: &Network) -> Result<(), String>;
    /// How the change on trial stands, and the management line now.
    fn trial(&mut self) -> (Trial, String);
    /// Keep the change on trial whatever the interface says.
    fn confirm_network(&mut self) -> Result<(), String>;
    /// Change the display zone and the time servers, and set the clock
    /// when the choice has one.
    fn set_time(&mut self, choice: &TimeChoice) -> Result<(), String>;
    /// The reference code history, newest first.
    fn history(&mut self) -> Vec<LogLine>;
    /// The system log, newest first.
    fn system_log(&mut self) -> Vec<LogLine>;
    /// Power the host off, or restart it.
    fn power(&mut self, restart: bool) -> Result<(), String>;
}

const KEYS: &str = "F1=Help   F3=Exit   F12=Cancel";

/// Sign-on. It ends with the user ID that signed on.
pub struct SignOn;

impl<E: Host + ?Sized> Panel<E, String> for SignOn {
    fn name(&self) -> &'static str {
        "tools-sign-on"
    }

    fn render(&self, env: &mut E) -> Screen {
        kit::form(
            "Service Tools Sign On",
            &env.system(),
            &[
                FormLine::new("Service tools user", users::ID_MAX, ""),
                FormLine::hidden("Password", 32),
            ],
            "Enter=Sign on   F1=Help   F3=Exit",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, String> {
        let id = fields.first().map(|f| f.trim()).unwrap_or_default();
        let password = fields.get(1).map(String::as_str).unwrap_or_default();
        if id.is_empty() {
            return Flow::Error("Type a service tools user ID.".into());
        }
        match env.sign_on(id, password) {
            Ok(id) => Flow::Exit(id),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Service Tools Sign On",
            lines: &[
                "Service Tools is the console of this host: its status, its",
                "load-source disks, its management network, its users, its clock,",
                "its logs and its power.",
                "",
                "Type a service tools user ID and its password, and press Enter.",
                "The password is not shown. User IDs are not case sensitive;",
                "passwords are.",
                "",
                "Five wrong passwords in a row disable a user ID for five",
                "minutes. Every sign-on that is refused is logged.",
                "",
                "A session that sees no key for ten minutes is signed off.",
            ],
        }
    }
}

/// What a confirmation does on F10.
type Action<E, O> = Box<dyn FnMut(&mut E) -> PanelFlow<E, O>>;

/// A confirmation: what will happen, F10 to do it.
pub struct Confirm<E: ?Sized, O> {
    name: &'static str,
    title: &'static str,
    lines: Vec<String>,
    help: Help,
    action: Action<E, O>,
}

impl<E: ?Sized, O> Confirm<E, O> {
    /// A confirmation that runs `action` on F10.
    pub fn new(
        name: &'static str,
        title: &'static str,
        lines: Vec<String>,
        help: Help,
        action: impl FnMut(&mut E) -> PanelFlow<E, O> + 'static,
    ) -> Self {
        Self {
            name,
            title,
            lines,
            help,
            action: Box::new(action),
        }
    }
}

impl<E: Named + ?Sized, O> Panel<E, O> for Confirm<E, O> {
    fn name(&self) -> &'static str {
        self.name
    }

    fn render(&self, env: &mut E) -> Screen {
        let mut lines = self.lines.clone();
        lines.push(String::new());
        lines.push("Press F10 to confirm, or F12 to go back.".into());
        kit::enterable(ui::text(
            self.title,
            &env.system(),
            &lines,
            None,
            "F1=Help   F10=Confirm   F12=Cancel",
        ))
    }

    fn enter(&mut self, _fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        Flow::Message("Press F10 to confirm, or F12 to go back.".into())
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        match key {
            10 => (self.action)(env),
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        self.help
    }
}

/// `Configure management network`.
pub struct NetworkPanel {
    entry: NetworkEntry,
}

impl NetworkPanel {
    /// The panel, showing the configuration in use.
    pub fn new<E: Host + ?Sized>(env: &mut E) -> Self {
        let (host, network) = env.settings();
        let links = env.links();
        Self {
            entry: NetworkEntry::of(host.name.as_deref().unwrap_or_default(), &network, &links),
        }
    }
}

impl<E: Host + ?Sized + 'static, O: 'static> Panel<E, O> for NetworkPanel {
    fn name(&self) -> &'static str {
        "tools-network"
    }

    fn render(&self, env: &mut E) -> Screen {
        let links = env.links();
        self.entry.screen(
            "Configure Management Network",
            &env.system(),
            &links,
            "Enter=Apply   F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        self.entry = NetworkEntry::take(fields);
        let links = env.links();
        let (name, network) = match self.entry.check(&links) {
            Ok(checked) => checked,
            Err(e) => return Flow::Error(e),
        };
        let (host, current) = env.settings();
        let renamed = host.name.as_deref() != Some(name.as_str());
        if renamed && let Err(e) = env.set_name(&name) {
            return Flow::Error(e);
        }
        if network == current {
            return if renamed {
                Flow::Message(format!("Host name changed to {name}."))
            } else {
                Flow::Message("Nothing was changed.".into())
            };
        }
        match env.apply_network(&network) {
            Ok(()) => Flow::Push(Box::new(TrialPanel)),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Configure Management Network",
            lines: &[
                "The fields show the configuration in use. Change what you want",
                "and press Enter; nothing restarts.",
                "",
                "Interface is the number of an interface in the list, or blank",
                "for the first that has carrier. Address is 1 for DHCP, or 2 for",
                "the IP address, prefix length and gateway typed here.",
                "",
                "A change of the network is tried before it is kept. It is kept",
                "as soon as the interface has carrier and an address with it. If",
                "it has neither after 30 seconds the configuration of before is",
                "put back, so a mistake cannot cut the host off for good. The",
                "next screen shows the trial; F10 there keeps the change at once.",
                "",
                "A change of the host name is made at once.",
            ],
        }
    }
}

/// The trial of a network change, watched.
struct TrialPanel;

impl<E: Host + ?Sized, O> Panel<E, O> for TrialPanel {
    fn name(&self) -> &'static str {
        "tools-network-trial"
    }

    fn render(&self, env: &mut E) -> Screen {
        let (trial, line) = env.trial();
        let mut lines = vec![format!("Management . . : {line}"), String::new()];
        let keys = match trial {
            Trial::Trying(left) => {
                lines.extend(kit::lines(&[
                    "The new configuration is in use and is being tried.",
                    "",
                    "It is kept as soon as the interface has carrier and an address.",
                ]));
                lines.push(format!(
                    "If it has not, the configuration of before is restored in {left} s."
                ));
                lines.push(String::new());
                lines.push(
                    "F10 keeps the new configuration now, whatever the interface says.".into(),
                );
                "F1=Help   F10=Keep"
            }
            Trial::Kept => {
                lines.push("The new configuration is kept. It is in use from every IPL on.".into());
                "Enter=Continue"
            }
            Trial::Restored(why) => {
                lines.push("The new configuration was not kept.".into());
                lines.push(format!("It had: {why}"));
                lines.push(String::new());
                lines.push("The configuration of before is in use again.".into());
                "Enter=Continue"
            }
            Trial::Unknown => {
                lines.push("The network service does not answer.".into());
                "Enter=Continue"
            }
        };
        kit::page("Configure Management Network", &env.system(), &lines, keys)
    }

    fn enter(&mut self, _fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        match env.trial().0 {
            Trial::Trying(_) => Flow::Stay,
            Trial::Kept => Flow::Unwind(
                2,
                Some(("Management network configuration changed.".into(), false)),
            ),
            Trial::Restored(_) | Trial::Unknown => Flow::Pop,
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        match (key, env.trial().0) {
            (10, Trial::Trying(_)) => match env.confirm_network() {
                Ok(()) => Flow::Message("The new configuration is kept.".into()),
                Err(e) => Flow::Error(e),
            },
            // A trial under way is not walked away from.
            (_, Trial::Trying(_)) => Flow::Stay,
            (3 | 12, _) => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Network change on trial",
            lines: &[
                "The configuration you typed is on the interface now. The line",
                "at the top is what the status screen shows of it.",
                "",
                "The host keeps the change, and writes it, as soon as the",
                "interface has carrier and an address. Nothing more is asked.",
                "",
                "If the interface has no carrier or no address when the seconds",
                "shown have passed, the configuration of before is restored and",
                "nothing was written.",
                "",
                "F10 keeps the change at once, without that check: for a network",
                "that is not connected yet.",
            ],
        }
    }

    fn live(&self) -> bool {
        true
    }
}

/// `Work with service tools user IDs`.
pub struct UsersPanel;

impl<E: Host + ?Sized + 'static, O: 'static> Panel<E, O> for UsersPanel {
    fn name(&self) -> &'static str {
        "tools-users"
    }

    fn render(&self, env: &mut E) -> Screen {
        let ids = env.user_ids();
        ui::list(
            "Work with Service Tools User IDs",
            &env.system(),
            "Type an option by a user ID, press Enter.  2=Change password  4=Remove",
            "Opt User ID",
            &ids,
            "F1=Help   F3=Exit   F6=Add   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        let ids = env.user_ids();
        let chosen = ids
            .iter()
            .zip(fields)
            .find(|(_, option)| !option.trim().is_empty());
        match chosen.map(|(id, option)| (id.clone(), option.trim())) {
            None => Flow::Error("Type an option by a user ID, or press F6 to add one.".into()),
            Some((id, "2")) => Flow::Push(Box::new(Password { id })),
            Some((id, "4")) => {
                let lines = vec![
                    format!("Service tools user ID {id} will be removed."),
                    String::new(),
                    "It can no longer sign on. The last user ID cannot be removed.".into(),
                ];
                Flow::Push(Box::new(Confirm::new(
                    "tools-user-remove",
                    "Confirm Remove of User ID",
                    lines,
                    Help {
                        title: "Confirm Remove of User ID",
                        lines: &[
                            "F10 removes the user ID named. It cannot sign on again;",
                            "to bring it back, add it with a new password.",
                            "",
                            "The last user ID is never removed: without one nobody",
                            "could sign on to Service Tools.",
                        ],
                    },
                    move |env: &mut E| match env.remove_user(&id) {
                        Ok(()) => Flow::Unwind(1, Some((format!("User ID {id} removed."), false))),
                        Err(e) => Flow::Unwind(1, Some((e, true))),
                    },
                )))
            }
            Some(_) => Flow::Error("Option must be 2 or 4.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        match key {
            6 => Flow::Push(Box::new(AddUser { id: String::new() })),
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Work with Service Tools User IDs",
            lines: &[
                "The user IDs that can sign on to Service Tools. Every one of",
                "them can do everything here.",
                "",
                "F6 adds a user ID. Type 2 by a user ID to give it a new",
                "password, which also enables an ID that wrong passwords have",
                "disabled. Type 4 by a user ID to remove it; the last one stays.",
                "",
                "Passwords are never shown and never kept: only a salted hash.",
            ],
        }
    }
}

struct AddUser {
    id: String,
}

impl<E: Host + ?Sized, O> Panel<E, O> for AddUser {
    fn name(&self) -> &'static str {
        "tools-user-add"
    }

    fn render(&self, env: &mut E) -> Screen {
        UserEntry::screen(&self.id, "Add Service Tools User ID", &env.system(), KEYS)
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        let entry = UserEntry::take(fields);
        self.id.clone_from(&entry.id);
        let (id, password) = match entry.check() {
            Ok(checked) => checked,
            Err(e) => return Flow::Error(e),
        };
        match env.add_user(&id, &password) {
            Ok(()) => Flow::Unwind(1, Some((format!("User ID {id} added."), false))),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Add Service Tools User ID",
            lines: &[
                "Type the new user ID, and its password twice. The rules are on",
                "the screen. What you type in the password fields is not shown.",
                "",
                "The new user can sign on at once, on the display and on the",
                "serial port, and can do everything in Service Tools.",
            ],
        }
    }
}

struct Password {
    id: String,
}

impl<E: Host + ?Sized, O> Panel<E, O> for Password {
    fn name(&self) -> &'static str {
        "tools-user-password"
    }

    fn render(&self, env: &mut E) -> Screen {
        let mut s = kit::form(
            "Change Password",
            &env.system(),
            &[
                FormLine::hidden("New password", 32),
                FormLine::hidden("New password again", 32),
            ],
            KEYS,
        );
        kit::put(&mut s, ui::BODY_ROW + 3, &format!("User ID: {}", self.id));
        for (i, rule) in users::RULES.iter().enumerate().skip(1) {
            kit::put(&mut s, ui::BODY_ROW + 4 + i, rule);
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        let entry = UserEntry {
            id: self.id.clone(),
            password: fields.first().cloned().unwrap_or_default(),
            again: fields.get(1).cloned().unwrap_or_default(),
        };
        let (id, password) = match entry.check() {
            Ok(checked) => checked,
            Err(e) => return Flow::Error(e),
        };
        match env.set_password(&id, &password) {
            Ok(()) => Flow::Unwind(1, Some((format!("Password of {id} changed."), false))),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Change Password",
            lines: &[
                "Type the new password of the user ID shown, twice. It is not",
                "shown as you type. The rules are on the screen.",
                "",
                "The old password stops working at once. A user ID that wrong",
                "passwords had disabled is enabled again.",
            ],
        }
    }
}

/// `Date and time`.
pub struct TimePanel {
    entry: TimeEntry,
}

impl TimePanel {
    /// The panel, showing the settings in use.
    pub fn new<E: Host + ?Sized>(env: &mut E) -> Self {
        Self {
            entry: TimeEntry::of(&env.settings().0),
        }
    }
}

impl<E: Host + ?Sized, O> Panel<E, O> for TimePanel {
    fn name(&self) -> &'static str {
        "tools-time"
    }

    fn render(&self, env: &mut E) -> Screen {
        let now = env.now();
        self.entry.screen(
            "Date and Time",
            &env.system(),
            now,
            "Enter=Apply   F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        self.entry = TimeEntry::take(fields);
        let choice = match self.entry.check() {
            Ok(choice) => choice,
            Err(e) => return Flow::Error(e),
        };
        match env.set_time(&choice) {
            Ok(()) => {
                // The clock is set once, not at every Enter.
                self.entry.date.clear();
                self.entry.time.clear();
                Flow::Message("Date and time settings changed.".into())
            }
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Date and Time",
            lines: &[
                "The host keeps its clock in UTC. The UTC offset says how dates",
                "and times are displayed on this console: +01:00 is one hour east",
                "of UTC. Blank displays UTC. Only fixed offsets are known; there",
                "is no change for summer time.",
                "",
                "Time servers: when given, the host keeps its clock in step with",
                "them. Blank the field to stop that.",
                "",
                "With no time servers you may set the clock by hand: type the",
                "date and the time as they are now, in the offset shown, and",
                "press Enter. Blank leaves the clock as it is.",
            ],
        }
    }
}

/// Which log a display shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Log {
    /// The reference code history.
    History,
    /// The system log.
    System,
}

/// `Display logs`: one log, newest first, paged.
pub struct LogPanel {
    log: Log,
    /// The least severity shown: 0, 1 or 2.
    least: u8,
    top: usize,
}

impl LogPanel {
    /// The display of `log`, from its newest line.
    #[must_use]
    pub fn new(log: Log) -> Self {
        Self {
            log,
            least: 0,
            top: 0,
        }
    }

    fn rows<E: Host + ?Sized>(&self, env: &mut E) -> Vec<String> {
        let lines = match self.log {
            Log::History => env.history(),
            Log::System => env.system_log(),
        };
        lines
            .into_iter()
            .filter(|l| l.severity >= self.least)
            .map(|l| {
                let mark = ["   ", "W  ", "E  "][usize::from(l.severity.min(2))];
                kit::fit(&format!("{}  {mark}{}", l.time, l.text), 78)
            })
            .collect()
    }
}

/// Rows of a log shown at once.
const LOG_ROWS: usize = 14;

impl<E: Host + ?Sized, O> Panel<E, O> for LogPanel {
    fn name(&self) -> &'static str {
        match self.log {
            Log::History => "tools-log-history",
            Log::System => "tools-log-system",
        }
    }

    fn render(&self, env: &mut E) -> Screen {
        let title = match self.log {
            Log::History => "Display Reference Code History",
            Log::System => "Display System Log",
        };
        let rows = self.rows(env);
        let (shown, foot) = kit::window(&rows, self.top, LOG_ROWS);
        let mut s = kit::form(
            title,
            &env.system(),
            &[FormLine::new(
                "Show: 1=All, 2=Warnings and errors, 3=Errors",
                1,
                (self.least + 1).to_string(),
            )],
            "Enter=Show   F1=Help   F3=Exit   F7=Newer   F8=Older   F12=Cancel",
        );
        for (i, row) in shown.iter().enumerate() {
            kit::put(&mut s, ui::BODY_ROW + 2 + i, row);
        }
        s.put(
            ui::BODY_ROW + 2 + LOG_ROWS,
            80 - foot.len(),
            &foot,
            console::Attr::WHITE,
        );
        s
    }

    fn enter(&mut self, fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        match kit::number(fields.first()) {
            Some(n @ 1..=3) => {
                self.least = u8::try_from(n - 1).unwrap_or(0);
                self.top = 0;
                Flow::Stay
            }
            _ => Flow::Error("Show must be 1, 2 or 3.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        match key {
            7 | 8 => {
                let len = self.rows(env).len();
                self.top = kit::scroll(self.top, key, len, LOG_ROWS);
                Flow::Stay
            }
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Display logs",
            lines: &[
                "The newest line is at the top. F8 shows older lines, F7 newer",
                "ones. Times are in the display zone.",
                "",
                "The reference code history is every code the host has posted,",
                "kept across IPLs and never cut. The system log is what PID 1",
                "and the services say; its oldest lines go when it is full.",
                "",
                "A line marked W is a warning or an attention code, one marked",
                "E an error. To see only those, type 2 or 3 in the Show field",
                "and press Enter.",
            ],
        }
    }
}

/// The host is shutting down: nothing more to do here.
struct Stopping {
    restart: bool,
}

impl<E: Named + ?Sized, O> Panel<E, O> for Stopping {
    fn name(&self) -> &'static str {
        "tools-stopping"
    }

    fn render(&self, env: &mut E) -> Screen {
        let what = if self.restart {
            "The host is restarting."
        } else {
            "The host is powering off."
        };
        kit::page(
            "Power Off or Restart",
            &env.system(),
            &kit::lines(&[what, "", "Its services are being stopped in order."]),
            "",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        Flow::Stay
    }

    fn function(&mut self, _key: u8, _fields: &[String], _env: &mut E) -> PanelFlow<E, O> {
        Flow::Stay
    }

    fn help(&self) -> Help {
        Help {
            title: "Power Off or Restart",
            lines: &["The host is stopping. This console ends with it."],
        }
    }
}

/// The confirmation of a power off or a restart.
#[must_use]
pub fn power<E: Host + ?Sized + 'static, O: 'static>(restart: bool) -> Confirm<E, O> {
    let (name, title, first) = if restart {
        (
            "tools-restart",
            "Confirm Restart",
            "The host will shut down in order and restart.",
        )
    } else {
        (
            "tools-power-off",
            "Confirm Power Off",
            "The host will shut down in order and power off.",
        )
    };
    Confirm::new(
        name,
        title,
        kit::lines(&[
            first,
            "",
            "Its services are stopped, its disks left clean. Every partition",
            "that runs on this host ends with it.",
        ]),
        Help {
            title: "Confirm power off or restart",
            lines: &[
                "F10 starts an orderly shutdown: the host's services end one",
                "after another, its data is written and its file system",
                "unmounted, and then the machine powers off or restarts.",
                "",
                "Partitions that run on the host end with it. Stop them from the",
                "management console first.",
                "",
                "F12 goes back and nothing happens.",
            ],
        },
        move |env: &mut E| match env.power(restart) {
            Ok(()) => Flow::Reset(Box::new(Stopping { restart })),
            Err(e) => Flow::Unwind(1, Some((e, true))),
        },
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::panel::Runner;
    use scrkit::panel::End;

    /// An appliance in memory.
    pub(crate) struct Fake {
        pub users: Vec<(String, String)>,
        pub host: HostFile,
        pub network: Network,
        pub trial: Trial,
        pub applied: Vec<Network>,
        pub power: Option<bool>,
        pub clock: Option<i64>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self {
                users: vec![("ADMIN".into(), "pass word 1".into())],
                host: HostFile {
                    name: Some("vs-01".into()),
                    ..HostFile::default()
                },
                network: Network::default(),
                trial: Trial::Unknown,
                applied: Vec::new(),
                power: None,
                clock: None,
            }
        }
    }

    impl Named for Fake {
        fn system(&self) -> String {
            self.host.name.clone().unwrap_or_default()
        }
    }

    impl Host for Fake {
        fn now(&self) -> i64 {
            1_794_488_700
        }
        fn sign_on(&mut self, id: &str, password: &str) -> Result<String, String> {
            let id = id.to_ascii_uppercase();
            if self.users.iter().any(|(i, p)| *i == id && p == password) {
                Ok(id)
            } else {
                Err("User ID or password not correct.".into())
            }
        }
        fn user_ids(&mut self) -> Vec<String> {
            self.users.iter().map(|(i, _)| i.clone()).collect()
        }
        fn add_user(&mut self, id: &str, password: &str) -> Result<(), String> {
            self.users.push((id.into(), password.into()));
            Ok(())
        }
        fn set_password(&mut self, id: &str, password: &str) -> Result<(), String> {
            let user = self.users.iter_mut().find(|(i, _)| i == id).ok_or("no")?;
            user.1 = password.into();
            Ok(())
        }
        fn remove_user(&mut self, id: &str) -> Result<(), String> {
            if self.users.len() == 1 {
                return Err("The last user ID cannot be removed.".into());
            }
            self.users.retain(|(i, _)| i != id);
            Ok(())
        }
        fn links(&mut self) -> Vec<LinkChoice> {
            vec![LinkChoice {
                name: "eth0".into(),
                mac: "52:54:00:12:34:56".into(),
                location: "P0000-B00-D04-F0-E0".into(),
                carrier: true,
            }]
        }
        fn settings(&mut self) -> (HostFile, Network) {
            (self.host.clone(), self.network.clone())
        }
        fn set_name(&mut self, name: &str) -> Result<(), String> {
            self.host.name = Some(name.into());
            Ok(())
        }
        fn apply_network(&mut self, network: &Network) -> Result<(), String> {
            self.applied.push(network.clone());
            self.trial = Trial::Trying(30);
            Ok(())
        }
        fn trial(&mut self) -> (Trial, String) {
            (self.trial.clone(), "none: no carrier on eth0".into())
        }
        fn confirm_network(&mut self) -> Result<(), String> {
            self.trial = Trial::Kept;
            Ok(())
        }
        fn set_time(&mut self, choice: &TimeChoice) -> Result<(), String> {
            self.host.time_zone.clone_from(&choice.zone);
            self.host.ntp_servers.clone_from(&choice.servers);
            self.clock = choice.clock;
            Ok(())
        }
        fn history(&mut self) -> Vec<LogLine> {
            (0..40)
                .map(|i| LogLine {
                    time: "2026-11-12 14:05".into(),
                    severity: u8::from(i % 10 == 0),
                    text: format!("C20010{i:02} Code {i}"),
                })
                .collect()
        }
        fn system_log(&mut self) -> Vec<LogLine> {
            Vec::new()
        }
        fn power(&mut self, restart: bool) -> Result<(), String> {
            self.power = Some(restart);
            Ok(())
        }
    }

    fn fields(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn message<O>(r: &Runner<Fake, O>, env: &mut Fake) -> String {
        let dump = r.screen(env).dump();
        dump.lines().nth(23).unwrap().trim().to_string()
    }

    #[test]
    fn sign_on_ends_with_the_user_or_says_no() {
        let mut env = Fake::default();
        let mut r: Runner<Fake, String> = Runner::new(Box::new(SignOn));
        assert!(r.screen(&mut env).dump().contains("Service Tools Sign On"));
        assert_eq!(r.enter(&fields(&["", ""]), &mut env), None);
        assert!(message(&r, &mut env).contains("Type a service tools user ID"));
        assert_eq!(r.enter(&fields(&["admin", "wrong"]), &mut env), None);
        assert_eq!(message(&r, &mut env), "User ID or password not correct.");
        // The password is in no cell of the screen.
        assert!(!r.screen(&mut env).dump().contains("wrong"));
        assert_eq!(
            r.enter(&fields(&["admin", "pass word 1"]), &mut env),
            Some(End::Outcome("ADMIN".into()))
        );
    }

    #[test]
    fn a_network_change_is_tried_and_kept_or_restored() {
        let mut env = Fake::default();
        let panel = NetworkPanel::new(&mut env);
        let mut r: Runner<Fake, ()> = Runner::new(Box::new(panel));
        let dhcp = ["vs-01", "", "1", "", "", "", "", ""];
        r.enter(&fields(&dhcp), &mut env);
        assert_eq!(message(&r, &mut env), "Nothing was changed.");
        let mut renamed = dhcp;
        renamed[0] = "vs-02";
        r.enter(&fields(&renamed), &mut env);
        assert_eq!(message(&r, &mut env), "Host name changed to vs-02.");
        assert!(env.applied.is_empty());

        let fixed = ["vs-02", "1", "2", "192.0.2.10", "24", "", "", ""];
        r.enter(&fields(&fixed), &mut env);
        assert_eq!(r.top(), "tools-network-trial");
        assert!(r.live());
        let shown = r.screen(&mut env).dump();
        assert!(shown.contains("restored in 30 s"));
        assert!(shown.contains("none: no carrier on eth0"));
        // It cannot be left while it is tried.
        r.function(12, &[], &mut env);
        r.enter(&[], &mut env);
        assert_eq!(r.top(), "tools-network-trial");
        env.trial = Trial::Restored("none: no carrier on eth0".into());
        assert!(r.screen(&mut env).dump().contains("was not kept"));
        r.enter(&[], &mut env);
        assert_eq!(r.top(), "tools-network");

        // Again, and kept by F10.
        r.enter(&fields(&fixed), &mut env);
        r.function(10, &[], &mut env);
        assert!(r.screen(&mut env).dump().contains("is kept"));
        // Back past the form, to the menu the form came from: here
        // there is none, so the form stays, with the word of it.
        assert_eq!(r.enter(&[], &mut env), None);
        assert_eq!(
            message(&r, &mut env),
            "Management network configuration changed."
        );
        assert_eq!(env.applied.len(), 2);
    }

    #[test]
    fn users_are_added_given_passwords_and_removed() {
        let mut env = Fake::default();
        let mut r: Runner<Fake, ()> = Runner::new(Box::new(UsersPanel));
        r.function(6, &[], &mut env);
        r.enter(&fields(&["second", "x", "x"]), &mut env);
        assert!(message(&r, &mut env).contains("at least 8"));
        r.enter(
            &fields(&["second", "other pass 2", "other pass 2"]),
            &mut env,
        );
        assert_eq!(message(&r, &mut env), "User ID SECOND added.");
        assert!(r.screen(&mut env).dump().contains("SECOND"));
        r.enter(&fields(&["", "2"]), &mut env);
        assert_eq!(r.top(), "tools-user-password");
        r.enter(&fields(&["new pass 33", "new pass 33"]), &mut env);
        assert_eq!(message(&r, &mut env), "Password of SECOND changed.");
        assert_eq!(env.users[1].1, "new pass 33");
        r.enter(&fields(&["4", ""]), &mut env);
        assert!(r.screen(&mut env).dump().contains("ADMIN will be removed"));
        r.function(10, &[], &mut env);
        assert_eq!(message(&r, &mut env), "User ID ADMIN removed.");
        // The last one stays.
        r.enter(&fields(&["4"]), &mut env);
        r.function(10, &[], &mut env);
        assert_eq!(message(&r, &mut env), "The last user ID cannot be removed.");
        assert_eq!(env.users.len(), 1);
        r.enter(&fields(&["9"]), &mut env);
        assert_eq!(message(&r, &mut env), "Option must be 2 or 4.");
    }

    #[test]
    fn the_time_the_logs_and_the_power() {
        let mut env = Fake::default();
        let panel = TimePanel::new(&mut env);
        let mut r: Runner<Fake, ()> = Runner::new(Box::new(panel));
        r.enter(&fields(&["+02:00", "", "2026-11-12", "15:05"]), &mut env);
        assert_eq!(message(&r, &mut env), "Date and time settings changed.");
        assert_eq!(env.host.time_zone, "UTC+02:00");
        assert_eq!(env.clock, Some(1_794_488_700));
        // The date is not left in the fields to be set again.
        assert!(!r.screen(&mut env).dump().contains("15:05  "));

        let mut r: Runner<Fake, ()> = Runner::new(Box::new(LogPanel::new(Log::History)));
        let first = r.screen(&mut env).dump();
        assert!(first.contains("C2001000 Code 0") && first.contains("More..."));
        assert!(first.contains("W  C2001000"));
        r.function(8, &[], &mut env);
        r.function(8, &[], &mut env);
        assert!(r.screen(&mut env).dump().contains("Bottom"));
        r.enter(&fields(&["2"]), &mut env);
        let warnings = r.screen(&mut env).dump();
        assert!(warnings.contains("Code 30") && !warnings.contains("Code 31"));
        r.enter(&fields(&["7"]), &mut env);
        assert_eq!(message(&r, &mut env), "Show must be 1, 2 or 3.");

        let mut r: Runner<Fake, ()> = Runner::new(Box::new(power(true)));
        assert!(r.screen(&mut env).dump().contains("Confirm Restart"));
        r.enter(&[], &mut env);
        assert_eq!(env.power, None);
        r.function(10, &[], &mut env);
        assert_eq!(env.power, Some(true));
        assert_eq!(r.top(), "tools-stopping");
        // Nothing leaves it.
        assert_eq!(r.function(3, &[], &mut env), None);
    }
}
