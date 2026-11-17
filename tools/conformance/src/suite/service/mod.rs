// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 7's suites: the partition service channel (`docs/SERVICE.md`),
//! on the test image, with QS/400 at the pinned commit as the system in
//! the partition. Everything is asked of the partition manager through
//! the test hooks' channel, as QS/MC's QSI will ask it: its list and
//! history, the shut down kinds, the panel functions, dumps, and the
//! console, which the hooks attach to as a viewer.
//!
//! - [`channel`]: codes, states, the manager started again, a silent
//!   system, a hostile one, one with no channel.
//! - [`shutdown`]: delayed and operating-system shut down, the host's
//!   power off, auto start's wait.
//! - [`panel`]: the panel functions, a terminal code held, dumps.
//! - [`console`]: the console port, the installer on it, Service Tools.

mod channel;
mod console;
mod panel;
mod shutdown;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use model::{ConsoleMode, Keylock};
use partd::proto::{ConsoleIn, ScreenCapture};
use partd::{Calls as _, Posted, State};
use qsipart::console::Key;
use qsvshook::proto::command;

use super::base::{Guest, Run, SuiteFn, channel as hooks};
use super::partition::manager::{self, Spec, partd, said};
use super::partition::qs400::{IPL_CODES, patience};
use super::partition::{QS400_MEMORY_MIB, UNIT, fresh_host, host, launch, qs400_image, unit};
use crate::qemu::session::Session;

/// The suites, in the order of Phase 7's handoff: its first, every
/// earlier suite on the new pin, is the build tool's, and so is its
/// last, the fuzz targets.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("service-codes", channel::codes),
    ("service-states", channel::states),
    ("service-delayed", shutdown::delayed),
    ("service-system", shutdown::system),
    ("service-panel", panel::functions),
    ("service-console", console::console),
    ("service-install-console", console::install),
    ("service-terminal", panel::terminal),
    ("service-dump-restart", panel::dump_restart),
    ("service-dumps", panel::dumps),
    ("service-manager-restart", channel::manager_restart),
    ("service-silent", channel::silent),
    ("service-hostile", channel::hostile),
    ("service-none", channel::none),
    ("service-host-power-off", shutdown::host_power_off),
    ("service-auto-wait", shutdown::auto_wait),
    ("service-tools", console::tools),
];

/// QS/400 in a partition with its console on the console port.
pub fn qs400_spec(keylock: Option<Keylock>) -> Spec {
    let mut spec = Spec::new(1, QS400_MEMORY_MIB, UNIT);
    spec.console = ConsoleMode::Partition;
    spec.keylock = keylock;
    spec
}

/// A host with a copy of the QS/400 image as its first further disk and
/// `more` after it; the partition manager answering.
pub fn qs400_host<'a>(run: &Run<'a>, more: &[PathBuf]) -> Result<Guest<'a>, String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let mut units = vec![unit(run, &image)?];
    units.extend_from_slice(more);
    host(run, &launch(run, &units)?)
}

/// Activate `id` with `spec` and wait for its system to report its IPL
/// complete: Running. The seconds it took.
pub fn up(guest: &mut Session, id: u32, spec: &Spec, deadline: Instant) -> Result<f64, String> {
    let began = Instant::now();
    manager::activate(guest, id, spec).map_err(|e| manager::refused(&e))?;
    manager::until(guest, id, State::Running, deadline)?;
    Ok(began.elapsed().as_secs_f64())
}

/// The partition's history.
pub fn history(guest: &mut Session, id: u32) -> Result<Vec<Posted>, String> {
    said(partd(guest)?.history(id))
}

/// The system's codes of a history, in order.
pub fn system_codes(history: &[Posted]) -> Vec<String> {
    history
        .iter()
        .filter(|p| p.source == partd::proto::Source::System)
        .map(|p| p.code.clone())
        .collect()
}

/// Whether `got` has every code of an IPL in order, each once.
pub fn ipl_in_order(got: &[String]) -> Result<(), String> {
    let mut at = 0;
    for want in IPL_CODES {
        let count = got.iter().filter(|c| *c == want).count();
        if count != 1 {
            return Err(format!("{want} is in the history {count} times: {got:?}"));
        }
        match got[at..].iter().position(|c| c == want) {
            Some(i) => at += i + 1,
            None => return Err(format!("{want} is out of order: {got:?}")),
        }
    }
    Ok(())
}

/// Type a line at the partition's serial service console.
pub fn serial(guest: &mut Session, id: u32, line: &str) -> Result<(), String> {
    super::partition::send_to(guest, id, line)
}

/// The partition's console screen now.
pub fn screen(guest: &mut Session, id: u32) -> Result<ScreenCapture, String> {
    said(partd(guest)?.screen(id))
}

/// Wait for the console to show any of `needles`; which, and the screen.
pub fn wait_screen(
    guest: &mut Session,
    id: u32,
    needles: &[&str],
    deadline: Instant,
) -> Result<(usize, String), String> {
    loop {
        if let Ok(s) = screen(guest, id)
            && let Some(i) = needles.iter().position(|n| s.text.contains(n))
        {
            return Ok((i, s.text));
        }
        if Instant::now() >= deadline {
            let now = screen(guest, id).map(|s| s.text).unwrap_or_default();
            return Err(format!("the console did not show {needles:?}:\n{now}"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// What a viewer attached through the hooks saw.
#[derive(Debug, Default)]
pub struct Viewed {
    /// `fact` lines: `serial`, `frames`, `updates`, `ended`, `second`,
    /// `takeover`.
    pub facts: Vec<(String, String)>,
    /// The screen as it was last shown, a row each.
    pub rows: Vec<String>,
}

impl Viewed {
    /// A fact's value.
    pub fn fact(&self, key: &str) -> &str {
        self.facts
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    }
}

/// Attach to `id`'s console as `mode` (`view`, `take`, `second`), send
/// `inputs`, watch `ms` more, detach.
pub fn view(
    guest: &mut Session,
    id: u32,
    mode: &str,
    ms: u64,
    inputs: &[ConsoleIn],
) -> Result<Viewed, String> {
    let mut line = format!("{} {id} {mode} {ms}", command::CONSOLE);
    if !inputs.is_empty() {
        line.push(' ');
        line.push_str(&hex::encode(ipc::encode(&inputs.to_vec())));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let lines = guest
        .channel(deadline)?
        .ask_within(&line, Duration::from_millis(ms) + Duration::from_secs(60))?;
    let mut viewed = Viewed::default();
    for l in lines {
        if let Some(fact) = l.strip_prefix("fact ") {
            let (k, v) = fact.split_once('=').unwrap_or((fact, ""));
            viewed.facts.push((k.to_string(), v.to_string()));
        } else if let Some(row) = l.strip_prefix("line ") {
            viewed.rows.push(row.to_string());
        } else if l.starts_with("line") {
            viewed.rows.push(String::new());
        }
    }
    Ok(viewed)
}

/// Keys for `text`, a character each.
pub fn typed(text: &str) -> Vec<ConsoleIn> {
    text.chars()
        .map(|ch| ConsoleIn::Key { key: Key::Char(ch) })
        .collect()
}

/// One key.
pub fn key(key: Key) -> ConsoleIn {
    ConsoleIn::Key { key }
}

/// The keys sent to `id`'s console, then the screen.
pub fn keys(guest: &mut Session, id: u32, inputs: &[ConsoleIn]) -> Result<Viewed, String> {
    view(guest, id, "view", 500, inputs)
}

/// QSECOFR's password after its first change: QS/400's own
/// conformance's.
pub const SECOFR_PASSWORD: &str = "Quartz2026";

/// Sign on at the console by its keys, whatever state QSECOFR's
/// password is in; leaves the first menu showing.
pub fn sign_on(guest: &mut Session, id: u32, deadline: Instant) -> Result<(), String> {
    for pw in [SECOFR_PASSWORD, "QSECOFR"] {
        let mut k = vec![key(Key::Home)];
        k.extend(typed("QSECOFR"));
        k.push(key(Key::FieldExit));
        k.extend(typed(pw));
        k.push(key(Key::FieldExit));
        k.push(key(Key::Enter));
        keys(guest, id, &k)?;
        let wanted = [
            "===>",
            "Change Password",
            "not correct",
            "Sign-on Information",
        ];
        let (i, _) = wait_screen(guest, id, &wanted, deadline)?;
        match i {
            0 => return Ok(()),
            1 => {
                let mut k = typed(pw);
                k.push(key(Key::FieldAdvance));
                k.extend(typed(SECOFR_PASSWORD));
                k.push(key(Key::FieldAdvance));
                k.extend(typed(SECOFR_PASSWORD));
                k.push(key(Key::Enter));
                keys(guest, id, &k)?;
                let (i, _) = wait_screen(guest, id, &["===>", "Sign-on Information"], deadline)?;
                if i == 1 {
                    keys(guest, id, &[key(Key::Enter)])?;
                }
                wait_screen(guest, id, &["===>"], deadline)?;
                return Ok(());
            }
            3 => {
                keys(guest, id, &[key(Key::Enter)])?;
                wait_screen(guest, id, &["===>"], deadline)?;
                return Ok(());
            }
            _ => {}
        }
    }
    Err(format!(
        "QSECOFR could not sign on:\n{}",
        screen(guest, id).map(|s| s.text).unwrap_or_default()
    ))
}

/// Through an attended IPL's screens on the console port to the IPL's
/// end: `IPL or Install the System`, `IPL Options`, `Set Major System
/// Options`.
pub fn attended(guest: &mut Session, id: u32, deadline: Instant) -> Result<String, String> {
    wait_screen(guest, id, &["IPL or Install the System"], deadline)?;
    let mut k = typed("1");
    k.push(key(Key::Enter));
    keys(guest, id, &k)?;
    let (_, options) = wait_screen(guest, id, &["IPL Options"], deadline)?;
    keys(guest, id, &[key(Key::Enter)])?;
    wait_screen(guest, id, &["Set Major System Options"], deadline)?;
    keys(guest, id, &[key(Key::Enter)])?;
    Ok(options)
}

/// The QS/400 checkout at the pinned commit, as cargo keeps it: where
/// its golden screens are.
fn qs400_checkout() -> Result<PathBuf, String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
    let rev = text
        .lines()
        .find(|l| l.starts_with("qsipart"))
        .and_then(|l| l.split("rev = \"").nth(1))
        .and_then(|r| r.get(..7))
        .ok_or("no pinned commit for qsipart in Cargo.toml")?
        .to_string();
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cargo")))
        .ok_or("no CARGO_HOME and no HOME")?;
    let checkouts = home.join("git/checkouts");
    for dir in std::fs::read_dir(&checkouts)
        .map_err(|e| e.to_string())?
        .flatten()
    {
        let candidate = dir.path().join(&rev);
        if dir
            .file_name()
            .to_string_lossy()
            .starts_with("quartz-systems-400-")
            && candidate.join("tools/conformance/golden").is_dir()
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "no checkout of QS/400 at {rev} under {}",
        checkouts.display()
    ))
}

/// `text` against QS/400's golden screen `dir/name`, its lines with the
/// trailing blanks dropped; only up to line `upto` when it is not 0,
/// and not the lines `skip` (from 1).
pub fn qs400_golden(
    dir: &str,
    name: &str,
    text: &str,
    skip: &[usize],
    upto: usize,
) -> Result<(), String> {
    let path = qs400_checkout()?
        .join("tools/conformance/golden")
        .join(dir)
        .join(format!("{name}.txt"));
    let want = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let norm = |t: &str| -> Vec<String> {
        let mut v: Vec<String> = t.lines().map(|l| l.trim_end().to_string()).collect();
        while v.last().is_some_and(String::is_empty) {
            v.pop();
        }
        v
    };
    let (got, want) = (norm(text), norm(&want));
    let mut last = got.len().max(want.len());
    if upto != 0 {
        last = last.min(upto);
    }
    for i in 0..last {
        if !skip.contains(&(i + 1)) && got.get(i) != want.get(i) {
            return Err(format!(
                "the console differs from QS/400's {dir}/{name}.txt at line {}:\n  got:  {:?}\n  want: {:?}",
                i + 1,
                got.get(i),
                want.get(i)
            ));
        }
    }
    Ok(())
}

/// A setting of the service channel changed on the host.
pub fn set_service(
    guest: &mut Session,
    change: impl FnOnce(&mut resource::Service),
) -> Result<(), String> {
    let mut settings = partd(guest)?.settings().map_err(|e| e.to_string())?;
    change(&mut settings.service);
    said(partd(guest)?.change_settings(settings))
}

/// What the hooks' channel answered `line` with, as lines.
pub fn ask(guest: &mut Session, line: &str) -> Result<Vec<String>, String> {
    hooks(guest)?.ask_within(line, Duration::from_secs(120))
}

/// How long a QS/400 IPL may take in a partition of a host that is
/// itself a guest.
pub fn ipl_patience(run: &Run<'_>) -> Instant {
    patience(run)
}
