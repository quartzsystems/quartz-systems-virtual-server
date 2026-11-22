// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 3, the host's network: the management network as before, the
//! network hardware and what each interface is, and the virtual switches
//! (made, changed, an uplink set or cleared, deleted, their status and
//! forwarding table). The partitions' adapters belong to their profiles
//! and are listed only: they are managed from QS/MC. The switch service
//! does the work; these panels ask it and show what it says.

use console::Screen;
use panels::tools::NetworkPanel;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use vswitch::{Bond, Switch, Uplink};
use vswitchd::{Calls as _, Interface, PortStatus, Refused, SwitchStatus};

use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

const SWITCHES: &str = "Work with Virtual Switches";
const MANAGED: &str = "Partitions' adapters are managed from QS/MC.";

/// The switch service's answer as a panel takes it.
fn asked<T>(answer: Result<Result<T, Refused>, ipc::WireError>) -> Result<T, String> {
    match answer {
        Err(e) => Err(format!("The switch service does not answer: {e}.")),
        Ok(Err(refused)) => Err(refused.first().map_or_else(
            || "The request was refused.".to_string(),
            ToString::to_string,
        )),
        Ok(Ok(v)) => Ok(v),
    }
}

fn statuses(env: &mut Env) -> Option<Vec<SwitchStatus>> {
    env.switches().switches().ok()
}

fn status(env: &mut Env, name: &str) -> Option<SwitchStatus> {
    statuses(env)?
        .into_iter()
        .find(|s| s.switch.as_ref().is_some_and(|w| w.name == name))
}

/// The keys of a display without its service: Enter goes back from a
/// display, and does nothing on a list or a choice.
const BACK: &str = "Enter=Continue   F3=Exit   F12=Cancel";
const STAY: &str = "F3=Exit   F12=Cancel";

fn no_service(title: &str, env: &mut Env, keys: &str) -> Screen {
    kit::page(
        title,
        &env.system(),
        &kit::lines(&["The switch service does not answer."]),
        keys,
    )
}

/// VLAN IDs as typed: numbers apart by spaces or commas.
pub fn parse_vlans(text: &str) -> Result<Vec<u16>, String> {
    let mut out = Vec::new();
    for word in text.split([' ', ',']).filter(|w| !w.is_empty()) {
        let v: u16 = word
            .parse()
            .map_err(|_| format!("{word} is not a VLAN ID: VLANs are 1 to 4094."))?;
        out.push(v);
    }
    Ok(out)
}

/// VLAN IDs as shown, cut to `width`.
fn vlan_text(vlans: &[u16], width: usize) -> String {
    let all: Vec<String> = vlans.iter().map(u16::to_string).collect();
    kit::fit(&all.join(" "), width)
}

/// The submenu of option 3.
pub struct Network;

impl Panel<Env, Outcome> for Network {
    fn name(&self) -> &'static str {
        "tools-network-menu"
    }

    fn render(&self, env: &mut Env) -> Screen {
        ui::menu(
            "Network",
            &env.system(),
            &[
                (1, "Configure management network"),
                (2, "Display network hardware"),
                (3, "Work with virtual switches"),
                (4, "Display partition adapters"),
            ],
            "Enter=Select   F1=Help   F3=Exit   F12=Cancel",
        )
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        match kit::number(fields.first()) {
            Some(1) => Flow::Push(Box::new(NetworkPanel::new(env))),
            Some(2) => Flow::Push(Box::new(Hardware)),
            Some(3) => Flow::Push(Box::new(Switches)),
            Some(4) => Flow::Push(Box::new(Adapters)),
            _ => Flow::Error("Selection not valid.".into()),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Network",
            lines: &[
                "1 configures the management network: the interface the host",
                "is managed through, its address, gateway and name servers.",
                "",
                "2 shows every network interface of the host by its location",
                "code, and what it is: the management interface, the uplink of",
                "a virtual switch, a member of an uplink's bond, or not used.",
                "",
                "3 works with the virtual switches partitions are on. 4 lists",
                "the partitions' adapters and their ports.",
            ],
        }
    }
}

fn hardware_row(i: &Interface) -> String {
    let speed = if i.speed_mbps == 0 {
        String::new()
    } else {
        format!("{}M", i.speed_mbps)
    };
    format!(
        "{} {} {} {:>6} {} {}",
        kit::fit(&i.location, 20),
        kit::fit(&i.mac, 17),
        kit::fit(&i.driver, 10),
        speed,
        if i.carrier { "Up  " } else { "Down" },
        kit::fit(&i.what, 14)
    )
}

/// `Display network hardware`.
pub struct Hardware;

impl Panel<Env, Outcome> for Hardware {
    fn name(&self) -> &'static str {
        "tools-network-hardware"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Ok(list) = env.switches().interfaces() else {
            return no_service("Display Network Hardware", env, BACK);
        };
        let mut text =
            vec!["Location             MAC address       Driver      Speed Link What".to_string()];
        if list.is_empty() {
            text.push("(The host has no network interface.)".into());
        }
        text.extend(list.iter().map(hardware_row));
        kit::page(
            "Display Network Hardware",
            &env.system(),
            &text,
            "Enter=Continue   F1=Help   F3=Exit   F5=Refresh   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
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
            title: "Display Network Hardware",
            lines: &[
                "Every network interface of the host: its location code, MAC",
                "address, driver, speed and whether it has a link, and what it",
                "is. The management interface is never a switch's uplink, and a",
                "switch's uplink is never the management interface.",
            ],
        }
    }
}

fn switch_row(s: &SwitchStatus) -> String {
    let Some(w) = &s.switch else {
        return String::new();
    };
    format!(
        "{} {} {:>5} {} {:>5}",
        kit::fit(&w.name, 10),
        vlan_text(&w.vlans, 28),
        w.mtu,
        kit::fit(&s.uplink_state, 9),
        s.ports
    )
}

/// `Work with virtual switches`.
pub struct Switches;

impl Panel<Env, Outcome> for Switches {
    fn name(&self) -> &'static str {
        "tools-switches"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Some(list) = statuses(env) else {
            return no_service(SWITCHES, env, STAY);
        };
        let rows: Vec<String> = list.iter().map(switch_row).collect();
        let mut s = ui::list(
            SWITCHES,
            &env.system(),
            "2=Change  4=Delete  5=Display  7=Set uplink  8=Clear uplink  9=Table",
            "Opt Name       VLANs                          MTU Uplink    Ports",
            &rows,
            "Enter=Select  F1=Help  F3=Exit  F5=Refresh  F6=Create  F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(
                &mut s,
                ui::BODY_ROW + 3,
                "(No virtual switch. F6 makes one.)",
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let Some(list) = statuses(env) else {
            return Flow::Error("The switch service does not answer.".into());
        };
        let chosen: Vec<(Switch, u32)> = list
            .iter()
            .zip(fields)
            .filter_map(|(s, f)| Some((s.switch.clone()?, kit::number(Some(f))?)))
            .collect();
        let [(w, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one switch.".into())
            };
        };
        let name = w.name.clone();
        match option {
            2 => Flow::Push(Box::new(Edit {
                old: Some(w.clone()),
            })),
            4 => Flow::Push(Box::new(Delete { name })),
            5 => Flow::Push(Box::new(Display { name })),
            7 => Flow::Push(Box::new(PickUplink { switch: name })),
            8 => match asked(env.switches().clear_uplink(name.clone())) {
                Ok(()) => {
                    Flow::Message(format!("Switch {name} is internal: its uplink is cleared."))
                }
                Err(e) => Flow::Error(e),
            },
            9 => Flow::Push(Box::new(Table { name })),
            _ => Flow::Error("Option not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            6 => Flow::Push(Box::new(Edit { old: None })),
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: SWITCHES,
            lines: &[
                "The host's virtual switches: each carries VLANs between the",
                "partitions on it and, through its uplink, to the outside. A",
                "switch with no uplink is internal.",
                "",
                "Type an option by a switch and press Enter: 2 changes its",
                "VLANs and MTU while partitions run on it; 5 shows its status;",
                "7 sets its uplink, or moves it; 8 clears it; 9 shows its",
                "forwarding table; 4 deletes a switch no profile names.",
                "",
                "F6 makes a switch.",
            ],
        }
    }
}

/// `Create virtual switch` and `Change virtual switch`.
struct Edit {
    old: Option<Switch>,
}

impl Panel<Env, Outcome> for Edit {
    fn name(&self) -> &'static str {
        if self.old.is_some() {
            "tools-switch-change"
        } else {
            "tools-switch-create"
        }
    }

    fn render(&self, env: &mut Env) -> Screen {
        let (title, name, vlans, mtu) = match &self.old {
            Some(w) => (
                "Change Virtual Switch",
                w.name.clone(),
                vlan_text(&w.vlans, 60),
                w.mtu.to_string(),
            ),
            None => (
                "Create Virtual Switch",
                String::new(),
                String::new(),
                "1500".into(),
            ),
        };
        let mut lines = Vec::new();
        if self.old.is_none() {
            lines.push(FormLine {
                label: "Switch name",
                len: 10,
                value: name,
                hidden: false,
            });
        }
        lines.push(FormLine {
            label: "VLANs",
            len: 40,
            value: vlans,
            hidden: false,
        });
        lines.push(FormLine {
            label: "MTU",
            len: 4,
            value: mtu,
            hidden: false,
        });
        let mut s = kit::form(
            title,
            &env.system(),
            &lines,
            "Enter=Apply   F1=Help   F12=Cancel",
        );
        let below = ui::BODY_ROW + lines.len() + 1;
        kit::put(
            &mut s,
            below,
            "VLANs: 1 to 4094, apart by spaces. MTU: 1280 to 9000.",
        );
        if let Some(w) = &self.old {
            kit::put(
                &mut s,
                below + 1,
                &format!("Switch {}: partitions on it go on running.", w.name),
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let at = usize::from(self.old.is_none());
        let vlans = match parse_vlans(fields.get(at).map_or("", |f| f.trim())) {
            Ok(v) => v,
            Err(e) => return Flow::Error(e),
        };
        let Ok(mtu) = fields.get(at + 1).map_or("", |f| f.trim()).parse::<u32>() else {
            return Flow::Error("The MTU is a number of bytes, 1280 to 9000.".into());
        };
        let done = if let Some(w) = &self.old {
            asked(env.switches().change(w.name.clone(), vlans, mtu)).map(|()| w.name.clone())
        } else {
            let name = fields.first().map_or("", |f| f.trim()).to_uppercase();
            asked(env.switches().create(name.clone(), vlans, mtu)).map(|_| name)
        };
        match done {
            Ok(name) => Flow::Unwind(1, Some((format!("Switch {name} is as given."), false))),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Create or Change Virtual Switch",
            lines: &[
                "A switch's name is how a profile's adapter names it: 1 to 10",
                "of A to Z, 0 to 9 and _, a letter first.",
                "",
                "VLANs are the ones it carries; an adapter is on one of them,",
                "and may be given more with their tags. A VLAN a profile uses",
                "stays. The MTU is its uplink's and every port's.",
                "",
                "A change applies at once and stops nothing that runs on it.",
            ],
        }
    }
}

/// `Confirm Delete of Switch`: the name typed in full.
struct Delete {
    name: String,
}

impl Panel<Env, Outcome> for Delete {
    fn name(&self) -> &'static str {
        "tools-switch-delete"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let mut s = kit::form(
            "Confirm Delete of Switch",
            &env.system(),
            &[FormLine {
                label: "Type it to confirm",
                len: 10,
                value: String::new(),
                hidden: false,
            }],
            "Enter=Confirm   F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 2,
            &format!(
                "Virtual switch {} is deleted, and its uplink given back.",
                self.name
            ),
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 3,
            "A switch a profile names or a partition is on is not deleted.",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 5,
            &format!("To go on, type: {}", self.name),
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        if fields.first().map_or("", |f| f.trim()) != self.name {
            return Flow::Error(format!("Type {} to confirm, or F12 to go back.", self.name));
        }
        match asked(env.switches().delete(self.name.clone(), false)) {
            Ok(()) => Flow::Unwind(
                1,
                Some((format!("Switch {} is deleted.", self.name), false)),
            ),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Confirm Delete of Switch",
            lines: &[
                "The switch is deleted when its name is typed in full and Enter",
                "is pressed. Its uplink's interfaces are given back, not used.",
                "",
                "F12 goes back and nothing is done.",
            ],
        }
    }
}

/// `Display virtual switch`.
struct Display {
    name: String,
}

impl Panel<Env, Outcome> for Display {
    fn name(&self) -> &'static str {
        "tools-switch-display"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Some(st) = status(env, &self.name) else {
            return no_service("Display Virtual Switch", env, BACK);
        };
        let Some(w) = st.switch.clone() else {
            return no_service("Display Virtual Switch", env, BACK);
        };
        let row = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 15));
        let mut text = vec![
            row("Switch", w.name.clone()),
            row("VLANs", vlan_text(&w.vlans, 58)),
            row("MTU", w.mtu.to_string()),
        ];
        match &w.uplink {
            None => text.push(row("Uplink", "None: internal".into())),
            Some(u) => {
                let mode = u.bond.map_or("one interface", Bond::word);
                text.push(row("Uplink", format!("{}, {mode}", st.uplink_state)));
                text.push(row(
                    "Native VLAN",
                    u.native_vlan
                        .map_or_else(|| "None".into(), |v| v.to_string()),
                ));
                for m in &st.members {
                    let link = if m.name.is_empty() {
                        "not found"
                    } else if m.carrier {
                        "up"
                    } else {
                        "down"
                    };
                    text.push(format!(
                        "    {} {} {link}",
                        kit::fit(&m.location, 22),
                        kit::fit(&m.name, 10)
                    ));
                }
            }
        }
        if !st.failure.is_empty() {
            text.push(kit::fit(&format!("Not built: {}", st.failure), 76));
        }
        text.push(row("Ports", st.ports.to_string()));
        kit::page(
            "Display Virtual Switch",
            &env.system(),
            &text,
            "Enter=Continue   F1=Help   F3=Exit   F5=Refresh   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
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
            title: "Display Virtual Switch",
            lines: &[
                "A switch's VLANs and MTU, its uplink and each interface of it",
                "by location code with its link, and how many partitions' ports",
                "are on it. An uplink that is Down stops outside traffic only:",
                "traffic between the partitions on the switch goes on.",
            ],
        }
    }
}

/// `Set uplink`: the interfaces, chosen by option 1.
struct PickUplink {
    switch: String,
}

impl Panel<Env, Outcome> for PickUplink {
    fn name(&self) -> &'static str {
        "tools-switch-uplink"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Ok(list) = env.switches().interfaces() else {
            return no_service("Set Uplink", env, STAY);
        };
        let rows: Vec<String> = list.iter().map(hardware_row).collect();
        let mut s = ui::list(
            "Set Uplink",
            &env.system(),
            &format!("1=Select for switch {}", self.switch),
            "Opt Location             MAC address       Driver      Speed Link What",
            &rows,
            "Enter=Select   F1=Help   F12=Cancel",
        );
        if rows.is_empty() {
            kit::put(
                &mut s,
                ui::BODY_ROW + 3,
                "(The host has no network interface.)",
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let Ok(list) = env.switches().interfaces() else {
            return Flow::Error("The switch service does not answer.".into());
        };
        let chosen: Vec<String> = list
            .iter()
            .zip(fields)
            .filter(|(_, f)| kit::number(Some(f)) == Some(1))
            .map(|(i, _)| i.location.clone())
            .collect();
        if chosen.is_empty() {
            return Flow::Error("Type 1 by one interface, or by several to bond them.".into());
        }
        Flow::Push(Box::new(UplinkForm {
            switch: self.switch.clone(),
            interfaces: chosen,
        }))
    }

    fn help(&self) -> Help {
        Help {
            title: "Set Uplink",
            lines: &[
                "Type 1 by the interface the switch is to reach the outside",
                "through, or by several to bond them. The management interface",
                "and another switch's are refused.",
            ],
        }
    }
}

/// The uplink's bond mode and native VLAN.
struct UplinkForm {
    switch: String,
    interfaces: Vec<String>,
}

impl Panel<Env, Outcome> for UplinkForm {
    fn name(&self) -> &'static str {
        "tools-switch-uplink-form"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let bonded = self.interfaces.len() > 1;
        let mut s = kit::form(
            "Set Uplink",
            &env.system(),
            &[
                FormLine {
                    label: "Bond mode",
                    len: 13,
                    value: if bonded {
                        "active-backup".into()
                    } else {
                        String::new()
                    },
                    hidden: false,
                },
                FormLine {
                    label: "Native VLAN",
                    len: 4,
                    value: String::new(),
                    hidden: false,
                },
            ],
            "Enter=Apply   F1=Help   F12=Cancel",
        );
        let mut row = ui::BODY_ROW + 3;
        kit::put(&mut s, row, &format!("Switch {}, uplink:", self.switch));
        for i in &self.interfaces {
            row += 1;
            kit::put(&mut s, row, &format!("    {i}"));
        }
        kit::put(
            &mut s,
            row + 2,
            "Bond mode: active-backup or 802.3ad; none for one interface.",
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let mode = fields.first().map_or("", |f| f.trim());
        let bond = if mode.is_empty() {
            None
        } else {
            let Some(b) = Bond::from_word(mode) else {
                return Flow::Error("Bond mode: active-backup or 802.3ad.".into());
            };
            Some(b)
        };
        let native = fields.get(1).map_or("", |f| f.trim());
        let native_vlan = if native.is_empty() {
            None
        } else {
            let Ok(v) = native.parse::<u16>() else {
                return Flow::Error(
                    "The native VLAN is one of the switch's VLANs, or none.".into(),
                );
            };
            Some(v)
        };
        let uplink = Uplink {
            interfaces: self.interfaces.clone(),
            bond,
            native_vlan,
        };
        match asked(env.switches().set_uplink(self.switch.clone(), uplink)) {
            Ok(()) => Flow::Unwind(
                2,
                Some((format!("Switch {} has its uplink.", self.switch), false)),
            ),
            Err(e) => Flow::Error(e),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Set Uplink",
            lines: &[
                "Several interfaces are bonded: active-backup needs nothing of",
                "the physical switch; 802.3ad needs LACP on its ports.",
                "",
                "The uplink is a trunk: every VLAN of the switch is tagged on",
                "it, but the native VLAN, which is carried untagged.",
            ],
        }
    }
}

/// `Display forwarding table` of a switch.
struct Table {
    name: String,
}

impl Panel<Env, Outcome> for Table {
    fn name(&self) -> &'static str {
        "tools-switch-table"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let rows = match asked(env.switches().fdb(self.name.clone())) {
            Ok(rows) => rows,
            Err(e) => {
                return kit::page(
                    "Display Forwarding Table",
                    &env.system(),
                    &[e],
                    "Enter=Continue   F3=Exit   F12=Cancel",
                );
            }
        };
        let mut text = vec![
            format!("Switch {}. {MANAGED}", self.name),
            String::new(),
            "MAC address       VLAN  Port                      How".to_string(),
        ];
        text.extend(rows.iter().take(kit::BODY_ROWS - 3).map(|r| {
            format!(
                "{} {:>4}  {} {}",
                kit::fit(&r.mac, 17),
                r.vlan,
                kit::fit(&r.port, 25),
                r.origin
            )
        }));
        if rows.len() > kit::BODY_ROWS - 3 {
            text.push(format!(
                "... and {} more.",
                rows.len() - (kit::BODY_ROWS - 3)
            ));
        }
        kit::page(
            "Display Forwarding Table",
            &env.system(),
            &text,
            "Enter=Continue   F1=Help   F3=Exit   F5=Refresh   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
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
            title: "Display Forwarding Table",
            lines: &[
                "Where the switch sends each address: a partition's port, put",
                "there for the adapter's address (static), the uplink, learned",
                "from the outside (learned), or the switch itself (local).",
            ],
        }
    }
}

fn adapter_row(p: &PortStatus) -> String {
    let mac = vswitch::mac::text(&p.spec.mac);
    format!(
        "{:>4} {:>4} {} {:>4} {} {} {:>9} {:>9}",
        p.spec.partition,
        p.spec.slot,
        kit::fit(&p.spec.switch, 10),
        p.spec.vlan,
        kit::fit(&mac, 17),
        if p.link { "Up  " } else { "Down" },
        p.counters.sent_packets,
        p.counters.received_packets
    )
}

/// `Display partition adapters`: read only.
pub struct Adapters;

impl Panel<Env, Outcome> for Adapters {
    fn name(&self) -> &'static str {
        "tools-adapters"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let Ok(ports) = env.switches().ports(String::new()) else {
            return no_service("Display Partition Adapters", env, BACK);
        };
        let mut text = vec![
            MANAGED.to_string(),
            String::new(),
            "Part Slot Switch     VLAN MAC address       Link      Sent  Received".to_string(),
        ];
        if ports.is_empty() {
            text.push("(No partition has a port open.)".into());
        }
        text.extend(ports.iter().take(kit::BODY_ROWS - 3).map(adapter_row));
        kit::page(
            "Display Partition Adapters",
            &env.system(),
            &text,
            "Enter=Continue   F1=Help   F3=Exit   F5=Refresh   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Pop
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
            title: "Display Partition Adapters",
            lines: &[
                "The running partitions' virtual Ethernet adapters: each one's",
                "partition and slot, its switch and port VLAN, its MAC address,",
                "whether it has link, and the frames it sent and was sent.",
                "",
                "Adapters belong to the partitions' profiles and are managed",
                "from QS/MC; nothing here changes them.",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlans_are_typed_apart_by_spaces_or_commas() {
        assert_eq!(parse_vlans("10 20,30").unwrap(), [10, 20, 30]);
        assert_eq!(parse_vlans("").unwrap(), Vec::<u16>::new());
        assert!(parse_vlans("10 x").is_err());
        assert_eq!(vlan_text(&[10, 20, 300], 6), kit::fit("10 20 300", 6));
    }
}
