// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `Power-On Settings` (option 9, 3): whether the partitions marked to
//! start do so with the host, how they end at a power off that does not
//! say, and how long each system is waited for. The partition manager
//! keeps them (`partitions.toml`'s `[power_on]`), as QSI's host family
//! does with bit 23.

use console::Screen;
use panels::ui::FormLine;
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use partd::Calls as _;
use resource::{PowerOff, PowerOn, Settings};

use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

fn client() -> partd::Client {
    partd::Client::new(partd::proto::socket(release::RUN_DIR))
}

/// What the three fields are filled with: the settings as they are.
fn fields(s: &Settings) -> [String; 3] {
    [
        if s.power_on.start_partitions {
            "Y"
        } else {
            "N"
        }
        .into(),
        match s.power_on.power_off {
            PowerOff::Delayed => "1".into(),
            PowerOff::Immediate => "2".into(),
        },
        s.service.host_timeout_secs.to_string(),
    ]
}

/// The settings a person typed: an error to show when they do not read.
pub fn read(fields: &[String]) -> Result<(PowerOn, u32), String> {
    let field = |i: usize| fields.get(i).map_or("", |f| f.trim());
    let start_partitions = match field(0).to_ascii_uppercase().as_str() {
        "Y" => true,
        "N" => false,
        _ => return Err("Type Y or N to start partitions with the host.".into()),
    };
    let power_off = match field(1) {
        "1" => PowerOff::Delayed,
        "2" => PowerOff::Immediate,
        _ => return Err("Type 1 for a delayed power off, 2 for immediate.".into()),
    };
    let delay = field(2)
        .parse()
        .map_err(|_| "The wait is a number of seconds, 10 to 3600.".to_string())?;
    Ok((
        PowerOn {
            start_partitions,
            power_off,
        },
        delay,
    ))
}

/// The panel.
#[derive(Debug, Default)]
pub struct PowerOnPanel;

impl Panel<Env, Outcome> for PowerOnPanel {
    fn name(&self) -> &'static str {
        "tools-power-on"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let now = client().settings().ok();
        let values = now.as_ref().map(fields).unwrap_or_default();
        let lines = [
            FormLine::new("Start partitions with the host (Y/N)", 1, values[0].clone()),
            FormLine::new("Power off: 1=Delayed 2=Immediate", 1, values[1].clone()),
            FormLine::new("Wait for each system, seconds", 4, values[2].clone()),
        ];
        let mut s = kit::form(
            "Power-On Settings",
            &env.system(),
            &lines,
            "F1=Help   F3=Exit   F12=Cancel",
        );
        if now.is_none() {
            kit::put(
                &mut s,
                panels::ui::BODY_ROW + 4,
                "(The partition manager does not answer.)",
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let (power_on, delay) = match read(fields) {
            Ok(v) => v,
            Err(e) => return Flow::Error(e),
        };
        match client().set_power_on(power_on, delay, env.who()) {
            Err(e) => Flow::Error(format!("The partition manager does not answer: {e}.")),
            Ok(Err(refused)) => Flow::Error(refused.first().map_or_else(
                || "The request was refused.".to_string(),
                ToString::to_string,
            )),
            Ok(Ok(())) => Flow::Unwind(1, Some(("Power-on settings kept.".into(), false))),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Power-On Settings",
            lines: &[
                "Y starts the partitions marked to start, each by its own",
                "auto start, when the host has started; N starts none.",
                "",
                "A power off that does not say ends the partitions as set:",
                "1 shuts each down through its own system, as its settings",
                "say, waiting for it; 2 ends each at once.",
                "",
                "The wait is how long each system is given, 10 to 3600",
                "seconds. A management console sees and changes the same.",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_read_as_the_settings_they_show() {
        let s = Settings::default();
        let shown = fields(&s);
        assert_eq!(shown, ["Y".to_string(), "1".into(), "300".into()]);
        let (on, delay) = read(&shown).unwrap();
        assert_eq!((on, delay), (s.power_on, 300));
        let typed = ["n".to_string(), "2".into(), " 60 ".into()];
        let (on, delay) = read(&typed).unwrap();
        assert!(!on.start_partitions);
        assert_eq!((on.power_off, delay), (PowerOff::Immediate, 60));
        assert!(read(&["X".to_string(), "1".into(), "60".into()]).is_err());
        assert!(read(&["Y".to_string(), "3".into(), "60".into()]).is_err());
        assert!(read(&["Y".to_string(), "1".into(), "x".into()]).is_err());
    }
}
