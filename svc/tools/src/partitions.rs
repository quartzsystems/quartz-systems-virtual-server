// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partitions, for looking at: ID, name, state, reference code,
//! and the processors and memory each holds now, as the partition
//! manager says. Nothing here acts on a partition: that is the
//! management console's.

use console::Screen;
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use partd::{State, Status};
use resource::Processors;

use crate::env::Env;
use crate::menu::Outcome;

/// Rows of the list on one screen.
const ROWS: usize = 13;

/// The list's heading.
pub const HEADING: &str = " ID  Name        State          Reference  Processors     Memory";

/// What a partition's processors are, in a few words.
fn processors(processors: &Processors) -> String {
    match processors {
        Processors::Dedicated { cores } => format!("{} dedicated", cores.len()),
        Processors::Shared {
            units, virtuals, ..
        } => format!("{units} on {virtuals} shr"),
    }
}

/// One partition's row.
pub fn row(p: &Status) -> String {
    let (held, memory) = match &p.current {
        Some(current) => (
            processors(&current.processors),
            format!("{} MB", current.memory_mib),
        ),
        None => ("-".to_string(), "-".to_string()),
    };
    format!(
        "{:>3}  {:<10}  {:<13}  {:<9}  {:<13}  {memory}",
        p.id,
        p.name,
        p.state.word(),
        if p.code.is_empty() { "-" } else { &p.code },
        held,
    )
    .trim_end()
    .to_string()
}

/// The line of the system status that counts them; `None` is a
/// partition manager that does not answer.
pub fn summary(partitions: Option<&[Status]>) -> String {
    match partitions {
        None => "the partition manager does not answer".to_string(),
        Some([]) => "none defined".to_string(),
        Some(all) => {
            let count = |state: State| all.iter().filter(|p| p.state == state).count();
            let mut words = vec![format!("{} defined", all.len())];
            for (state, word) in [
                (State::Running, "running"),
                (State::Starting, "starting"),
                (State::ShuttingDown, "shutting down"),
                (State::Error, "in error"),
            ] {
                if count(state) > 0 {
                    words.push(format!("{} {word}", count(state)));
                }
            }
            words.join(", ")
        }
    }
}

/// `Display Partitions`: from the system status, F11.
#[derive(Default)]
pub struct Partitions {
    top: usize,
}

type Step = PanelFlow<Env, Outcome>;

impl Panel<Env, Outcome> for Partitions {
    fn name(&self) -> &'static str {
        "tools-partitions"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let partitions = list();
        let rows: Vec<String> = partitions
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(row)
            .collect();
        let (shown, foot) = kit::window(&rows, self.top, ROWS);
        let mut lines = vec![HEADING.to_string()];
        lines.extend(shown);
        lines.push(match partitions {
            Some(_) => format!("{foot:>76}"),
            None => "The partition manager does not answer.".to_string(),
        });
        while lines.len() < ROWS + 3 {
            lines.push(String::new());
        }
        lines.push("Partitions are managed from the management console, QS/MC.".into());
        kit::page(
            "Display Partitions",
            &env.system(),
            &lines,
            "F1=Help   F3=Exit   F7=Backward   F8=Forward   F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            7 | 8 => {
                let len = list().map_or(0, |all| all.len());
                self.top = kit::scroll(self.top, key, len, ROWS);
                Flow::Stay
            }
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Partitions",
            lines: &[
                "The partitions of this host as they are now; the display",
                "follows them. Nothing can be done to a partition here.",
                "",
                "State is Not activated, Starting, Running, Shutting down or",
                "Error. Reference is the code the partition posted last, as the",
                "host saw it; the partition's own codes are on its console.",
                "",
                "Processors is what the partition holds now: whole cores when",
                "they are dedicated, or its processing units and its virtual",
                "processors when they are shared. Memory is what it holds now.",
                "A partition that is not activated holds nothing.",
                "",
                "A partition is created, changed, activated and shut down from",
                "the management console, QS/MC.",
            ],
        }
    }

    fn live(&self) -> bool {
        true
    }
}

/// The partitions, in ID order; `None` when the partition manager
/// does not answer.
pub fn list() -> Option<Vec<Status>> {
    use partd::Calls as _;
    partd::Client::new(partd::proto::socket(release::RUN_DIR))
        .list()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::Units;
    use resource::Current;

    fn partition(id: u32, name: &str, state: State, current: Option<Current>) -> Status {
        Status {
            id,
            name: name.into(),
            state,
            code: if state == State::NotActivated {
                String::new()
            } else {
                "C2005090".into()
            },
            text: String::new(),
            current,
            profile: "NORMAL".into(),
            last_end: String::new(),
            message: None,
            pid: 0,
        }
    }

    #[test]
    fn a_row_has_what_the_partition_holds_now() {
        let dedicated = Current {
            processors: Processors::Dedicated { cores: vec![6, 7] },
            memory_mib: 2048,
        };
        let shared = Current {
            processors: Processors::Shared {
                pool: "DEFAULT".into(),
                units: Units(50),
                virtuals: 1,
                capped: true,
                weight: 0,
            },
            memory_mib: 1024,
        };
        let all = [
            partition(1, "PROD", State::Running, Some(dedicated)),
            partition(12, "TESTSHARED", State::Starting, Some(shared)),
            partition(3, "DEV", State::NotActivated, None),
        ];
        assert_eq!(
            row(&all[0]),
            "  1  PROD        Running        C2005090   2 dedicated    2048 MB"
        );
        assert_eq!(
            row(&all[1]),
            " 12  TESTSHARED  Starting       C2005090   0.50 on 1 shr  1024 MB"
        );
        assert_eq!(
            row(&all[2]),
            "  3  DEV         Not activated  -          -              -"
        );
        // The heading's columns are the rows'.
        for (word, in_row) in [("State", "Running"), ("Reference", "C2005090")] {
            assert_eq!(HEADING.find(word), row(&all[0]).find(in_row));
        }
        assert!(all.iter().all(|p| row(p).len() <= 76));
        assert_eq!(summary(Some(&all)), "3 defined, 1 running, 1 starting");
        assert_eq!(summary(Some(&[])), "none defined");
        assert!(summary(None).contains("does not answer"));
    }
}
