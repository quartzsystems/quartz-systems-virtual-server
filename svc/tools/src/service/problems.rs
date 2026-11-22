// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Option 8, 1: the problems, one shown whole with its resources and
//! its analysis log, one closed; and option 7, 3: the problem log.

use console::Screen;
use panels::ui::{self, FormLine};
use panels::{Flow, Help, Named, Panel, PanelFlow, kit};
use problem::proto::Calls as _;
use problem::{Filter, Status, Summary};

use super::{down, said, when};
use crate::env::Env;
use crate::menu::Outcome;

type Step = PanelFlow<Env, Outcome>;

const TITLE: &str = "Work with Problems";
/// The rows a list shows at a time.
const ROWS: usize = kit::BODY_ROWS - 4;

/// Every problem the log keeps, newest first.
fn all(env: &mut Env, open_only: bool) -> Result<Vec<Summary>, String> {
    let filter = Filter {
        status: open_only.then_some(Status::Open),
        ..Filter::default()
    };
    let mut out = Vec::new();
    let mut before = None;
    loop {
        let page = env
            .problems()
            .list(filter.clone(), before, 500)
            .map_err(|e| down("problem service", &e))?;
        out.extend(page.problems);
        match page.more {
            Some(m) => before = Some(m),
            None => return Ok(out),
        }
    }
}

/// `Open`, `Cleared` or `Closed`.
fn state(s: &Summary) -> &'static str {
    match (s.status, s.holds) {
        (Status::Closed, _) => "Closed",
        (Status::Open, true) => "Open",
        (Status::Open, false) => "Cleared",
    }
}

fn row(env: &Env, s: &Summary) -> String {
    format!(
        "{:>6}  {:<7}  {}  {:<8}  {}",
        s.number,
        state(s),
        kit::fit(&when(env, s.last_seen), 16),
        s.refcode.code,
        kit::fit(&s.description, 30)
    )
}

/// `Work with Problems`.
#[derive(Default)]
pub struct Problems {
    top: usize,
    /// Rows at the last draw: a page key needs not make the list again.
    seen: std::cell::Cell<usize>,
    every: bool,
}

impl Panel<Env, Outcome> for Problems {
    fn name(&self) -> &'static str {
        "tools-problems"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let list = match all(env, !self.every) {
            Ok(l) => l,
            Err(e) => {
                return kit::page(TITLE, &env.system(), &[e], "F1=Help   F3=Exit   F12=Cancel");
            }
        };
        let rows: Vec<String> = list.iter().map(|s| row(env, s)).collect();
        self.seen.set(rows.len());
        let (shown, foot) = kit::window(&rows, self.top, ROWS);
        let mut s = ui::list(
            TITLE,
            &env.system(),
            "4=Close  5=Display",
            "Opt Number  Status   Last seen         Code      Description",
            &shown,
            if self.every {
                "Enter=Select  F1=Help  F3=Exit  F5=Refresh  F11=Open only  F12=Cancel"
            } else {
                "Enter=Select  F1=Help  F3=Exit  F5=Refresh  F11=Show all  F12=Cancel"
            },
        );
        kit::put(&mut s, ui::BODY_ROW + 4 + ROWS, &foot);
        if rows.is_empty() {
            kit::put(
                &mut s,
                ui::BODY_ROW + 3,
                if self.every {
                    "(The problem log is empty.)"
                } else {
                    "(No problem is open. F11 shows the closed ones too.)"
                },
            );
        }
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let list = match all(env, !self.every) {
            Ok(l) => l,
            Err(e) => return Flow::Error(e),
        };
        let shown: Vec<&Summary> = list.iter().skip(self.top).take(ROWS).collect();
        let chosen: Vec<(&Summary, u32)> = shown
            .into_iter()
            .zip(fields)
            .filter_map(|(s, f)| Some((s, kit::number(Some(f))?)))
            .collect();
        let [(s, option)] = chosen.as_slice() else {
            return if chosen.is_empty() {
                Flow::Stay
            } else {
                Flow::Error("Type an option by one problem.".into())
            };
        };
        match option {
            4 if s.status == Status::Closed => {
                Flow::Error(format!("Problem {} is closed already.", s.number))
            }
            4 => Flow::Push(Box::new(Close {
                number: s.number,
                holds: s.holds,
            })),
            5 => Flow::Push(Box::new(Display {
                number: s.number,
                top: 0,
                seen: std::cell::Cell::default(),
            })),
            _ => Flow::Error("Option not valid.".into()),
        }
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            11 => {
                self.every = !self.every;
                self.top = 0;
                Flow::Stay
            }
            7 | 8 => {
                let len = self.seen.get();
                self.top = kit::scroll(self.top, key, len, ROWS);
                Flow::Stay
            }
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: TITLE,
            lines: &[
                "Each problem is a condition a person should look at, opened",
                "from a reference code the host posted: newest first, the",
                "open ones unless F11 shows all. Cleared means its condition",
                "no longer holds; a person still closes it.",
                "",
                "Type 5 by a problem to see it whole: what it is about, what",
                "to do, and its analysis log. Type 4 to close it. F7 and F8",
                "page through the list.",
            ],
        }
    }
}

/// `Display Problem`.
struct Display {
    number: u64,
    top: usize,
    /// Rows at the last draw: a page key needs not make the list again.
    seen: std::cell::Cell<usize>,
}

/// The lines of one problem.
fn detail(env: &Env, p: &problem::Problem) -> Vec<String> {
    let s = &p.summary;
    let line = |label: &str, value: String| format!("{} {value}", kit::dotted(label, 20));
    let mut out = vec![
        line("Problem", s.number.to_string()),
        line(
            "Status",
            match (s.status, s.holds) {
                (Status::Closed, _) => {
                    format!("Closed by {} at {}", s.closed_by, when(env, s.closed_at))
                }
                (Status::Open, true) => "Open; its condition holds".into(),
                (Status::Open, false) => "Open; its condition was cleared".into(),
            },
        ),
        line(
            "Reference code",
            format!("{} {}", s.refcode.code, s.refcode.text),
        ),
        line("Source", s.source.words()),
        line("First seen", when(env, s.first_seen)),
        line(
            "Last seen",
            format!("{}, {} time(s)", when(env, s.last_seen), s.count),
        ),
        line("Message", s.msgid.clone()),
        String::new(),
    ];
    out.extend(wrap(&s.description, 78));
    if let Some(m) = qvsmsg::find(&s.msgid) {
        out.push(String::new());
        out.extend(wrap(m.help, 78));
    }
    out.push(String::new());
    out.push("Resources:".into());
    if p.resources.is_empty() {
        out.push("  the host".into());
    }
    for r in &p.resources {
        out.push(format!("  {}", problem::record::said(r)));
    }
    out.push(String::new());
    out.push("Analysis log:".into());
    for l in &p.log {
        out.push(format!("  {}  {}", when(env, l.time), l.text));
    }
    out
}

/// `text` in lines of at most `width`, broken at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

impl Panel<Env, Outcome> for Display {
    fn name(&self) -> &'static str {
        "tools-problem"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let lines = match env.problems().get(self.number) {
            Ok(Ok(p)) => detail(env, &p),
            Ok(Err(issued)) => vec![said(&issued)],
            Err(e) => vec![down("problem service", &e)],
        };
        self.seen.set(lines.len());
        let (shown, foot) = kit::window(&lines, self.top, kit::BODY_ROWS - 1);
        let mut text = shown;
        text.push(foot);
        kit::page(
            "Display Problem",
            &env.system(),
            &text,
            "F1=Help  F3=Exit  F7=Back  F8=Forward  F10=Close problem  F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn function(&mut self, key: u8, _fields: &[String], env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            7 | 8 => {
                let len = self.seen.get();
                self.top = kit::scroll(self.top, key, len, kit::BODY_ROWS - 1);
                Flow::Stay
            }
            10 => match env.problems().get(self.number) {
                Ok(Ok(p)) if p.summary.status == Status::Closed => {
                    Flow::Error(format!("Problem {} is closed already.", self.number))
                }
                Ok(Ok(p)) => Flow::Push(Box::new(Close {
                    number: self.number,
                    holds: p.summary.holds,
                })),
                Ok(Err(issued)) => Flow::Error(said(&issued)),
                Err(e) => Flow::Error(down("problem service", &e)),
            },
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Problem",
            lines: &[
                "One problem whole: its reference code, whose it is, when it",
                "was first and last seen and how often, what it is and what",
                "to do, the resources it is about with their location codes",
                "and serial numbers, and its analysis log.",
                "",
                "F7 and F8 page. F10 closes it.",
            ],
        }
    }
}

/// `Close Problem`.
struct Close {
    number: u64,
    holds: bool,
}

impl Panel<Env, Outcome> for Close {
    fn name(&self) -> &'static str {
        "tools-problem-close"
    }

    fn render(&self, env: &mut Env) -> Screen {
        if !self.holds {
            let mut s = kit::page(
                "Close Problem",
                &env.system(),
                &[
                    format!(
                        "Problem {} is closed: its condition no longer holds.",
                        self.number
                    ),
                    String::new(),
                    "Press Enter to close it, or F12 to go back.".into(),
                ],
                "Enter=Confirm   F1=Help   F12=Cancel",
            );
            s.cursor = (ui::BODY_ROW, 1);
            return s;
        }
        let mut s = kit::form(
            "Close Problem",
            &env.system(),
            &[FormLine {
                label: "Close it anyway (Y=Yes)",
                len: 1,
                value: String::new(),
                hidden: false,
            }],
            "Enter=Confirm   F1=Help   F12=Cancel",
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 2,
            &format!("The condition of problem {} still holds.", self.number),
        );
        kit::put(
            &mut s,
            ui::BODY_ROW + 3,
            "Close it anyway only when it is understood and will be dealt with.",
        );
        s
    }

    fn enter(&mut self, fields: &[String], env: &mut Env) -> Step {
        let anyway = fields
            .first()
            .is_some_and(|f| f.trim().eq_ignore_ascii_case("Y"));
        if self.holds && !anyway {
            return Flow::Error("Type Y to close it anyway, or F12 to go back.".into());
        }
        let by = env.user.clone();
        match env.problems().close(self.number, anyway, by) {
            Ok(Ok(_)) => Flow::Unwind(
                1,
                Some((format!("Problem {} is closed.", self.number), false)),
            ),
            Ok(Err(issued)) => Flow::Error(said(&issued)),
            Err(e) => Flow::Error(down("problem service", &e)),
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Close Problem",
            lines: &[
                "A closed problem stays closed, with who closed it. When its",
                "condition is seen again, a new problem is opened.",
            ],
        }
    }
}

/// Option 7, 3: `Display Problem Log`, every problem as a line.
#[derive(Default)]
pub struct Log {
    top: usize,
    /// Rows at the last draw: a page key needs not make the list again.
    seen: std::cell::Cell<usize>,
}

impl Panel<Env, Outcome> for Log {
    fn name(&self) -> &'static str {
        "tools-log-problems"
    }

    fn render(&self, env: &mut Env) -> Screen {
        let lines = match all(env, false) {
            Ok(list) => {
                let mut out = vec![
                    "Number  Opened            Last seen         Count  Status   Code".to_string(),
                ];
                for s in &list {
                    out.push(format!(
                        "{:>6}  {}  {}  {:>5}  {:<7}  {}",
                        s.number,
                        kit::fit(&when(env, s.first_seen), 16),
                        kit::fit(&when(env, s.last_seen), 16),
                        s.count,
                        state(s),
                        s.refcode.code
                    ));
                    out.push(format!("        {}", kit::fit(&s.description, 70)));
                }
                out
            }
            Err(e) => vec![e],
        };
        self.seen.set(lines.len());
        let (shown, foot) = kit::window(&lines, self.top, kit::BODY_ROWS - 1);
        let mut text = shown;
        text.push(foot);
        kit::page(
            "Display Problem Log",
            &env.system(),
            &text,
            "F1=Help  F3=Exit  F7=Back  F8=Forward  F12=Cancel",
        )
    }

    fn enter(&mut self, _fields: &[String], _env: &mut Env) -> Step {
        Flow::Stay
    }

    fn function(&mut self, key: u8, _fields: &[String], _env: &mut Env) -> Step {
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            7 | 8 => {
                let len = self.seen.get();
                self.top = kit::scroll(self.top, key, len, kit::BODY_ROWS - 1);
                Flow::Stay
            }
            _ => Flow::Stay,
        }
    }

    fn help(&self) -> Help {
        Help {
            title: "Display Problem Log",
            lines: &[
                "Every problem the log keeps, newest first, open or closed:",
                "when it was opened and last seen, how often, and its code.",
                "Option 8, 1 works with them.",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_wraps_at_its_spaces() {
        let w = wrap("one two three four five", 9);
        assert_eq!(w, ["one two", "three", "four five"]);
        assert!(wrap("", 9).is_empty());
    }
}
