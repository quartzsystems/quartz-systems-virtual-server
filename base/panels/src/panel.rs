// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A panel is one screen and what Enter and the function keys do on
//! it, as in QS/400's screen kit, over the kit's own stack. A panel of
//! an appliance has two things more: help text, shown on F1, and a
//! tick, for a screen that shows something that moves (a progress, a
//! status, a countdown).

use console::Screen;
use scrkit::panel::{End, Flow, Stack};
use scrkit::ui;

/// What the panels of one application ask of their environment, at
/// the least: the name on the `System:` line.
pub trait Named {
    /// The system's name.
    fn system(&self) -> String;
}

/// The help of a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Help {
    /// The panel's title.
    pub title: &'static str,
    /// The text, a line each.
    pub lines: &'static [&'static str],
}

/// One screen's behavior over an environment `E`, ending with `O`.
pub trait Panel<E: ?Sized, O> {
    /// Name, for logs and golden files.
    fn name(&self) -> &'static str;
    /// The screen, without the message line.
    fn render(&self, env: &mut E) -> Screen;
    /// Enter with the input fields in order.
    fn enter(&mut self, fields: &[String], env: &mut E) -> PanelFlow<E, O>;
    /// A function key, with the input fields in order. F3 exits and
    /// F12 cancels, unless overridden. F1 never arrives: it is help.
    fn function(&mut self, key: u8, fields: &[String], env: &mut E) -> PanelFlow<E, O> {
        let _ = (fields, env);
        match key {
            3 => Flow::PopAll,
            12 => Flow::Pop,
            _ => Flow::Stay,
        }
    }
    /// What F1 shows.
    fn help(&self) -> Help;
    /// Whether the screen is drawn again every second while no key
    /// comes. Such a panel has no field the operator types in.
    fn live(&self) -> bool {
        false
    }
    /// A second passed on a live panel.
    fn tick(&mut self, env: &mut E) -> PanelFlow<E, O> {
        let _ = env;
        Flow::Stay
    }
}

/// What a [`Panel`] answers a key with.
pub type PanelFlow<E, O> = Flow<Box<dyn Panel<E, O>>, O>;

/// The keys of a help display.
pub const HELP_KEYS: &str = "Enter=Continue   F12=Cancel";

/// A stack of [`Panel`]s, with help.
pub struct Runner<E: ?Sized, O> {
    stack: Stack<Box<dyn Panel<E, O>>>,
    helping: bool,
}

impl<E: Named + ?Sized, O> Runner<E, O> {
    /// Start with one panel.
    #[must_use]
    pub fn new(root: Box<dyn Panel<E, O>>) -> Self {
        Self {
            stack: Stack::new(root),
            helping: false,
        }
    }

    /// The screen to show now.
    pub fn screen(&self, env: &mut E) -> Screen {
        if self.helping {
            let help = self.stack.top().help();
            let lines: Vec<String> = help.lines.iter().map(ToString::to_string).collect();
            return crate::kit::enterable(ui::text(
                &format!("Help: {}", help.title),
                &env.system(),
                &lines,
                None,
                HELP_KEYS,
            ));
        }
        let mut s = self.stack.top().render(env);
        self.stack.put_message(&mut s);
        s
    }

    /// Name of the panel on top; `help` while its help is shown.
    #[must_use]
    pub fn top(&self) -> &'static str {
        if self.helping {
            "help"
        } else {
            self.stack.top().name()
        }
    }

    /// Panels on the stack.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.depth()
    }

    /// Whether the screen is to be drawn again every second.
    #[must_use]
    pub fn live(&self) -> bool {
        !self.helping && self.stack.top().live()
    }

    /// Enter was pressed with these field contents.
    pub fn enter(&mut self, fields: &[String], env: &mut E) -> Option<End<O>> {
        if self.helping {
            self.helping = false;
            return None;
        }
        let flow = self.stack.top_mut().enter(fields, env);
        self.stack.apply(flow)
    }

    /// A function key was pressed.
    pub fn function(&mut self, key: u8, fields: &[String], env: &mut E) -> Option<End<O>> {
        if self.helping {
            self.helping = false;
            return None;
        }
        if key == 1 {
            self.helping = true;
            return None;
        }
        let flow = self.stack.top_mut().function(key, fields, env);
        self.stack.apply(flow)
    }

    /// A second passed with no key. A panel that has nothing to say
    /// keeps its message line.
    pub fn tick(&mut self, env: &mut E) -> Option<End<O>> {
        if !self.live() {
            return None;
        }
        match self.stack.top_mut().tick(env) {
            Flow::Stay => None,
            flow => self.stack.apply(flow),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Env(&'static str);

    impl Named for Env {
        fn system(&self) -> String {
            self.0.into()
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Done {
        Finished,
    }

    struct Counter {
        depth: u32,
        ticks: u32,
    }

    impl Panel<Env, Done> for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }
        fn render(&self, env: &mut Env) -> Screen {
            ui::text(
                "Counter",
                &env.system(),
                &[format!("{} {}", self.depth, self.ticks)],
                None,
                "",
            )
        }
        fn enter(&mut self, fields: &[String], _env: &mut Env) -> PanelFlow<Env, Done> {
            match fields.first().map(String::as_str) {
                Some("push") => Flow::Push(Box::new(Counter {
                    depth: self.depth + 1,
                    ticks: 0,
                })),
                Some("bad") => Flow::Error("no".into()),
                _ => Flow::Stay,
            }
        }
        fn help(&self) -> Help {
            Help {
                title: "Counter",
                lines: &["It counts."],
            }
        }
        fn live(&self) -> bool {
            self.depth == 1
        }
        fn tick(&mut self, _env: &mut Env) -> PanelFlow<Env, Done> {
            self.ticks += 1;
            if self.ticks == 3 {
                Flow::Exit(Done::Finished)
            } else {
                Flow::Stay
            }
        }
    }

    fn line(r: &Runner<Env, Done>, env: &mut Env, n: usize) -> String {
        r.screen(env).dump().lines().nth(n).unwrap().to_string()
    }

    #[test]
    fn f1_shows_the_help_and_any_key_leaves_it() {
        let mut env = Env("SYSTEM1");
        let mut r = Runner::new(Box::new(Counter { depth: 0, ticks: 0 }));
        assert_eq!(r.function(1, &[], &mut env), None);
        assert_eq!(r.top(), "help");
        assert!(line(&r, &mut env, 0).contains("Help: Counter"));
        assert!(line(&r, &mut env, 3).contains("It counts."));
        assert!(line(&r, &mut env, 22).contains(HELP_KEYS));
        // F3 on the help leaves the help, not the application.
        assert_eq!(r.function(3, &[], &mut env), None);
        assert_eq!(r.top(), "counter");
        assert_eq!(r.function(3, &[], &mut env), Some(End::Left));
    }

    #[test]
    fn a_live_panel_ticks_and_keeps_its_message() {
        let mut env = Env("SYSTEM1");
        let mut r = Runner::new(Box::new(Counter { depth: 0, ticks: 0 }));
        // The first panel is not live: no tick reaches it.
        assert!(!r.live());
        assert_eq!(r.tick(&mut env), None);
        r.enter(&["push".into()], &mut env);
        assert!(r.live());
        r.enter(&["bad".into()], &mut env);
        assert_eq!(r.tick(&mut env), None);
        assert!(line(&r, &mut env, 23).starts_with("no"));
        assert!(line(&r, &mut env, 3).contains("1 1"));
        assert_eq!(r.tick(&mut env), None);
        assert_eq!(r.tick(&mut env), Some(End::Outcome(Done::Finished)));
    }
}
