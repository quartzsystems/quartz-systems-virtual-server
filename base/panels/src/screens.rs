// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The console under the panels: QS/400's `console` on the
//! framebuffer with its keyboards, on a serial terminal, or on both at
//! once, and the loop that shows a stack of panels and gives it the
//! keys. A serial port is put in raw mode and read here; its bytes
//! become the console's keys.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use console::ansi::{AnsiDisplay, KeyDecoder};
use console::{Command, Console, Identity, Input, Key, PointerEvent, Scale, Screen};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::termios::{OptionalActions, tcgetattr, tcsetattr};
use scrkit::panel::End;

use crate::panel::{Named, Runner};

/// How often a live panel is drawn again, and the idle time looked at.
const TICK: Duration = Duration::from_secs(1);
/// How long a lone Esc waits for the rest of a sequence.
const ESCAPE_WAIT: Duration = Duration::from_millis(150);

/// A serial port, raw: nothing is echoed, no line is waited for.
pub struct SerialPort {
    file: File,
}

impl SerialPort {
    /// Open the port at `path` and put it in raw mode.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut mode = tcgetattr(&file)?;
        mode.make_raw();
        tcsetattr(&file, OptionalActions::Now, &mode)?;
        Ok(Self { file })
    }

    /// A handle to write to the port.
    pub fn writer(&self) -> io::Result<File> {
        self.file.try_clone()
    }

    /// Read the port for ever on a thread of its own. Keys go to the
    /// console that [`SerialKeys::attach`] names; while there is none,
    /// Enter is counted for [`SerialKeys::wait_enter`].
    pub fn keys(&self) -> io::Result<SerialKeys> {
        let mut file = self.file.try_clone()?;
        let sink: Arc<Mutex<Option<Sender<Command>>>> = Arc::default();
        let (enter_tx, enter) = mpsc::channel();
        let to = Arc::clone(&sink);
        std::thread::spawn(move || {
            let mut decoder = KeyDecoder::new();
            let mut byte = [0u8; 1];
            loop {
                let key = if decoder.pending() && !readable(&file, ESCAPE_WAIT) {
                    decoder.flush()
                } else {
                    match file.read(&mut byte) {
                        Ok(1) => decoder.feed(byte[0]),
                        Ok(_) => {
                            // No terminal is there; look again later.
                            std::thread::sleep(Duration::from_secs(1));
                            None
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => None,
                        Err(_) => return,
                    }
                };
                let Some(key) = key else { continue };
                let sink = to.lock().unwrap_or_else(PoisonError::into_inner);
                match &*sink {
                    Some(console) => {
                        let _ = console.send(Command::Virtual(key));
                    }
                    None if key == Key::Enter => {
                        let _ = enter_tx.send(());
                    }
                    None => {}
                }
            }
        });
        Ok(SerialKeys { sink, enter })
    }
}

fn readable(file: &File, within: Duration) -> bool {
    let timeout = Timespec {
        tv_sec: 0,
        tv_nsec: i64::from(within.subsec_nanos()),
    };
    let mut fds = [PollFd::from_borrowed_fd(file.as_fd(), PollFlags::IN)];
    poll(&mut fds, Some(&timeout)).is_ok_and(|n| n > 0)
}

/// The keys of a serial port.
pub struct SerialKeys {
    sink: Arc<Mutex<Option<Sender<Command>>>>,
    enter: Receiver<()>,
}

impl SerialKeys {
    /// Send the port's keys to `screens` from now on.
    pub fn attach(&self, screens: &Screens) {
        *self.sink.lock().unwrap_or_else(PoisonError::into_inner) = Some(screens.keys.clone());
    }

    /// Send the port's keys nowhere: only Enter is counted.
    pub fn detach(&self) {
        *self.sink.lock().unwrap_or_else(PoisonError::into_inner) = None;
        while self.enter.try_recv().is_ok() {}
    }

    /// Wait for Enter on the port while no console has its keys.
    /// False when `within` passed without one.
    pub fn wait_enter(&self, within: Duration) -> bool {
        self.enter.recv_timeout(within).is_ok()
    }
}

/// What a console is shown on.
pub struct Displays {
    /// The framebuffer, with the keyboards.
    pub framebuffer: bool,
    /// The scale of the framebuffer's cells.
    pub scale: Scale,
    /// A serial terminal.
    pub serial: Option<File>,
}

/// The console is gone: its thread ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gone;

/// One console: screens go in, Enter and the function keys come out.
pub struct Screens {
    show: Sender<Command>,
    inputs: Receiver<Input>,
    keys: Sender<Command>,
    last_key: Arc<Mutex<Instant>>,
    described: String,
    framebuffer: bool,
}

/// A way to show a screen on a console from another thread.
#[derive(Clone)]
pub struct Shower {
    show: Sender<Command>,
}

impl Shower {
    /// Show a screen.
    pub fn show(&self, screen: Screen) {
        let _ = self.show.send(Command::ShowScreen(screen));
    }
}

impl Screens {
    /// Open a console on `displays`, speaking for `identity`.
    pub fn open(identity: Identity, displays: Displays) -> Self {
        let (mut console, status) =
            Console::open_scaled(identity, displays.framebuffer, displays.scale);
        let mut described = match status {
            Ok(()) => console.describe(),
            Err(e) => e.to_string(),
        };
        let framebuffer = console.has_display();
        if let Some(serial) = displays.serial {
            console.add_display(Box::new(AnsiDisplay::new(serial).named("serial")));
            described.push_str(", serial terminal");
        }
        // A click puts the cursor in a field or presses a key's label.
        console.set_pointer(true);
        let (show, commands) = mpsc::channel();
        let (input_tx, inputs) = mpsc::channel();
        std::thread::spawn(move || console.run(&commands, input_tx));

        // Every key passes here, so the last one's time is known.
        let (keys, typed) = mpsc::channel::<Command>();
        let last_key = Arc::new(Mutex::new(Instant::now()));
        let (to, stamp) = (show.clone(), Arc::clone(&last_key));
        std::thread::spawn(move || {
            while let Ok(key) = typed.recv() {
                *stamp.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
                let Some(key) = rolled(key) else { continue };
                if to.send(key).is_err() {
                    break;
                }
            }
        });
        if displays.framebuffer {
            crate::keyboards::watch(&keys);
        }
        Self {
            show,
            inputs,
            keys,
            last_key,
            described,
            framebuffer,
        }
    }

    /// Whether the console is on a framebuffer.
    pub fn has_framebuffer(&self) -> bool {
        self.framebuffer
    }

    /// A way to show screens from another thread.
    pub fn shower(&self) -> Shower {
        Shower {
            show: self.show.clone(),
        }
    }

    /// What the console is shown on, for a log.
    pub fn describe(&self) -> &str {
        &self.described
    }

    /// Show a screen.
    pub fn show(&self, screen: Screen) {
        let _ = self.show.send(Command::ShowScreen(screen));
    }

    /// Show a screen, and how long the console took to draw it: a dump
    /// asked after it is answered once it is drawn. `None` when the
    /// console did not answer within [`DRAW_WAIT`].
    pub fn show_drawn(&self, screen: Screen) -> Option<Duration> {
        let asked = Instant::now();
        self.show(screen);
        let (tx, drawn) = mpsc::channel();
        let _ = self.show.send(Command::Dump(tx));
        drawn.recv_timeout(DRAW_WAIT).ok().map(|_| asked.elapsed())
    }

    /// How long ago the last key was typed.
    pub fn idle(&self) -> Duration {
        self.last_key
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }

    /// Count from now, as if a key had been typed.
    pub fn touch(&self) {
        *self.last_key.lock().unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    /// Enter or a function key, when one comes within `within`.
    pub fn input(&self, within: Duration) -> Result<Option<Input>, Gone> {
        match self.inputs.recv_timeout(within) {
            Ok(input) => Ok(Some(input)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Gone),
        }
    }

    /// Forget keys typed before now.
    pub fn drain(&self) {
        while self.inputs.try_recv().is_ok() {}
    }
}

/// `KEY_PAGEUP` and `KEY_PAGEDOWN` of an event device.
const PAGE_UP: u16 = 104;
const PAGE_DOWN: u16 = 109;

/// A key as the panels get it: Page Up and Page Down, which `console`'s
/// editor drops, become the roll keys, and so does the display's wheel;
/// their release is nothing.
fn rolled(command: Command) -> Option<Command> {
    let roll = |forward| {
        Some(Command::Virtual(Key::Function(if forward {
            crate::panel::ROLL_FORWARD
        } else {
            crate::panel::ROLL_BACK
        })))
    };
    match command {
        Command::Key(ev) if ev.code == PAGE_UP || ev.code == PAGE_DOWN => {
            if ev.pressed {
                roll(ev.code == PAGE_DOWN)
            } else {
                None
            }
        }
        Command::Pointer(PointerEvent::Wheel { up }) => roll(!up),
        Command::Virtual(Key::PageUp) => roll(false),
        Command::Virtual(Key::PageDown) => roll(true),
        other => Some(other),
    }
}

/// The longest a screen is waited for to be drawn.
const DRAW_WAIT: Duration = Duration::from_secs(10);
/// A screen made and drawn in longer than this is logged.
const SLOW: Duration = Duration::from_millis(250);

/// Log a screen that was slow to make (the panel's work: its files and
/// the services it asks) or to draw (the console and its displays).
fn slow(panel: &str, made: Duration, drawn: Option<Duration>) {
    let Some(drawn) = drawn else {
        println!(
            "screen {panel}: made in {} ms, not drawn within {} s",
            made.as_millis(),
            DRAW_WAIT.as_secs()
        );
        return;
    };
    if made + drawn >= SLOW {
        println!(
            "screen {panel} slow: made in {} ms, drawn in {} ms",
            made.as_millis(),
            drawn.as_millis()
        );
    }
}

/// How a run of panels ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended<O> {
    /// Back from the first panel.
    Left,
    /// A panel ended it.
    Outcome(O),
    /// No key came for the idle time.
    Idle,
}

/// Show the panels of `runner` on `screens` and give them the keys,
/// until they end or no key comes for `idle`.
pub fn run<E: Named + ?Sized, O>(
    screens: &Screens,
    runner: &mut Runner<E, O>,
    env: &mut E,
    idle: Option<Duration>,
) -> Ended<O> {
    screens.touch();
    loop {
        let asked = Instant::now();
        let screen = runner.screen(env);
        let made = asked.elapsed();
        let drawn = screens.show_drawn(screen);
        slow(runner.top(), made, drawn);
        // Wait until a key or a tick changes what is shown.
        let end = loop {
            match screens.input(TICK) {
                Err(Gone) => return Ended::Left,
                Ok(Some(Input::Enter(fields))) => break runner.enter(&fields, env),
                Ok(Some(Input::Function(key, fields))) => {
                    break runner.function(key, &fields, env);
                }
                Ok(None) => {
                    if idle.is_some_and(|idle| screens.idle() >= idle) {
                        return Ended::Idle;
                    }
                    if runner.live() {
                        break runner.tick(env);
                    }
                }
            }
        };
        match end {
            Some(End::Left) => return Ended::Left,
            Some(End::Outcome(outcome)) => return Ended::Outcome(outcome),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::keyboard::KeyEvent;

    #[allow(clippy::needless_pass_by_value)] // takes what `rolled` gives
    fn function(command: Option<Command>) -> Option<u8> {
        match command {
            Some(Command::Virtual(Key::Function(n))) => Some(n),
            _ => None,
        }
    }

    #[test]
    fn page_keys_become_the_roll_keys() {
        let ev = |code, pressed| Command::Key(KeyEvent { code, pressed });
        let back = crate::panel::ROLL_BACK;
        let forward = crate::panel::ROLL_FORWARD;
        assert_eq!(function(rolled(ev(PAGE_UP, true))), Some(back));
        assert_eq!(function(rolled(ev(PAGE_DOWN, true))), Some(forward));
        assert!(rolled(ev(PAGE_DOWN, false)).is_none());
        assert_eq!(function(rolled(Command::Virtual(Key::PageUp))), Some(back));
        assert_eq!(
            function(rolled(Command::Virtual(Key::PageDown))),
            Some(forward)
        );
        // The wheel away is Page Up; a click passes to the console.
        let wheel = |up| Command::Pointer(PointerEvent::Wheel { up });
        assert_eq!(function(rolled(wheel(true))), Some(back));
        assert_eq!(function(rolled(wheel(false))), Some(forward));
        let click = Command::Pointer(PointerEvent::Button { pressed: true });
        assert!(matches!(rolled(click), Some(Command::Pointer(_))));
        // Any other key passes as it is.
        assert!(matches!(rolled(ev(28, true)), Some(Command::Key(_))));
    }
}
