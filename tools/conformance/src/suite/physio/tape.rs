// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Section 1: QS/400 against the virtual tape drive. Two installed
//! QS/400 partitions and the media library; every command typed at the
//! system's Command Entry, every volume mounted and unmounted from the
//! host, as an operator would. What each step showed is the detail, and
//! `docs/QS400_GUEST.md` keeps it.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use model::profile::{MediaDevice, Profile};
use partd::{Calls as _, State};
use poold::proto::Calls as _;
use qsipart::console::Key;

use crate::qemu::image::{blank, sparse_copy};
use crate::qemu::session::Session;
use crate::suite::base::{Guest, Run, channel};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::partition::{fresh_host, host, launch, qs400_image, unit};
use crate::suite::service::{
    ipl_patience, key, keys, qs400_spec, screen, sign_on, typed, wait_screen,
};
use crate::suite::storage::{disk_names, pool_until, poold};

/// The tape device's slot in both profiles.
const SLOT: u32 = 3;
/// The name QS/400 gives the first tape drive it finds.
const DEVICE: &str = "TAP01";
/// The title of QS/400's Command Entry.
const COMMAND_ENTRY: &str = "Command Entry";

/// The texts of QS/400's messages that end a tape or save command
/// (its `runtime/msgf/msgs/qcpfmsg.toml`), and of a command refused.
const FAILURES: &[&str] = &[
    "ended because of an error",           // CPF6708
    "cannot be written: it is protected",  // CPF6768
    "could not be read as expected",       // CPF6772
    "is not the one asked for",            // CPF6720
    "no further volume was available",     // CPF6718
    "is not ready",                        // CPF6760
    "The save ended",                      // CPF3794
    "Nothing named was found in the save", // CPF3780
    // The same by identifier, which QS/400 puts before its own text
    // where that is not the catalogue's (`docs/QS400_GUEST.md`).
    "CPF6708",
    "CPF6768",
    "CPF6772",
    "CPF6720",
    "CPF6718",
    "CPF6760",
    "CPF3794",
    "CPF3780",
    "not found",
    "not allowed",
    "not valid",
];

/// The two partitions, the media library's mount point and the host.
struct Rig<'a> {
    guest: Guest<'a>,
    first: u32,
    second: u32,
    mount: String,
}

/// A host with two QS/400 units and a pool disk made the media
/// repository, the volumes `(name, identifier, MiB)` in it, and the
/// partitions FIRST and SECOND defined.
fn rig<'a>(run: &Run<'a>, volumes: &[(&str, &str, u64)]) -> Result<Rig<'a>, String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let a = unit(run, &image)?;
    let b = run.file("unit2.img");
    sparse_copy(&image, &b)?;
    let pool = run.file("pool0.img");
    blank(&pool, 2 << 30)?;
    let mut l = launch(run, &[a, b, pool])?;
    l.memory_mb = 8192;
    let mut guest = host(run, &l)?;
    let names = disk_names(&mut guest, 3)?;
    crate::suite::storage::said(poold(&mut guest)?.create_pool(
        "MEDIA".into(),
        "none".into(),
        vec![names[2].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    let p = pool_until(&mut guest, "MEDIA", run.deadline(), |p| !p.mount.is_empty())?;
    crate::suite::storage::said(poold(&mut guest)?.designate("MEDIA".into()))?;
    for (name, identifier, mib) in volumes {
        crate::suite::storage::said(poold(&mut guest)?.create_volume(
            (*name).into(),
            (*identifier).into(),
            *mib,
        ))?;
    }
    let first = manager::partition(&mut guest, "FIRST")?;
    let second = manager::partition(&mut guest, "SECOND")?;
    Ok(Rig {
        guest,
        first,
        second,
        mount: p.mount,
    })
}

/// QS/400 on `device` with its console on the console port and a tape
/// device in [`SLOT`] holding `volume` at activation.
fn profile(device: &str, volume: Option<&str>) -> Profile {
    let mut spec = qs400_spec(None);
    spec.backing = vec![device.to_string()];
    spec.name = "TAPE".into();
    let mut p = spec.profile();
    p.tape = vec![MediaDevice {
        slot: SLOT,
        media: volume.map(Into::into),
    }];
    p
}

/// Activate `id` with `p`, wait for its system, sign on and open
/// Command Entry.
fn start(guest: &mut Session, id: u32, p: Profile, run: &Run<'_>) -> Result<(), String> {
    let name = p.name.clone();
    said(partd(guest)?.put_profile(id, p))?;
    said(partd(guest)?.activate(id, name, None, None))?;
    let deadline = ipl_patience(run);
    manager::until(guest, id, State::Running, deadline)?;
    wait_screen(guest, id, &["Sign On"], deadline)?;
    sign_on(guest, id, deadline)?;
    enter(guest, id, "call qcmd")?;
    wait_screen(guest, id, &[COMMAND_ENTRY], deadline)?;
    Ok(())
}

/// Type `line` on the command line and press Enter.
fn enter(guest: &mut Session, id: u32, line: &str) -> Result<(), String> {
    let mut k = vec![key(Key::Home), key(Key::FieldExit), key(Key::Home)];
    k.extend(typed(line));
    k.push(key(Key::Enter));
    keys(guest, id, &k).map(|_| ())
}

/// A text no message has, sent to Command Entry's messages before a
/// command so that only what follows it is the command's.
fn mark(guest: &mut Session, id: u32) -> Result<String, String> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    // A data area of a name of its own in the job's QTEMP: its command
    // echoes the name, and its completion message names it again.
    let m = format!("QM{:04}", NEXT.fetch_add(1, Ordering::Relaxed));
    enter(
        guest,
        id,
        &format!("CRTDTAARA DTAARA(QTEMP/{m}) TYPE(*CHAR) LEN(1)"),
    )?;
    // The command's echo has the name too; the message is a second line.
    let deadline = Instant::now() + Duration::from_mins(1);
    loop {
        if screen(guest, id)?.text.matches(&m).count() >= 2 {
            return Ok(m);
        }
        if Instant::now() >= deadline {
            let s = screen(guest, id)?.text;
            return Err(format!("CRTDTAARA did not show `{m}`:\n{s}"));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// What the screen has after the last line with `mark`; the whole
/// screen when the mark is gone.
fn after<'s>(text: &'s str, mark: &str) -> &'s str {
    text.rfind(mark).map_or(text, |at| &text[at + mark.len()..])
}

/// Which of `needles` the screen shows after `mark`.
fn shown(text: &str, mark: &str, needles: &[&str]) -> Option<usize> {
    let tail = after(text, mark);
    needles.iter().position(|n| tail.contains(n))
}

/// Run `command` at Command Entry and wait until one of `needles`
/// shows below it: which, and the screen.
fn command(
    guest: &mut Session,
    id: u32,
    command: &str,
    needles: &[&str],
    within: Duration,
) -> Result<(usize, String), String> {
    let m = mark(guest, id)?;
    enter(guest, id, command)?;
    until(guest, id, &m, needles, within)
        .map_err(|s| format!("`{command}` showed none of {needles:?}:\n{s}"))
}

/// Wait until one of `needles` shows after `mark`; the screen as the
/// error when none does by `within`.
fn until(
    guest: &mut Session,
    id: u32,
    mark: &str,
    needles: &[&str],
    within: Duration,
) -> Result<(usize, String), String> {
    let deadline = Instant::now() + within;
    loop {
        if let Ok(s) = screen(guest, id)
            && let Some(i) = shown(&s.text, mark, needles)
        {
            return Ok((i, s.text));
        }
        if Instant::now() >= deadline {
            return Err(screen(guest, id).map(|s| s.text).unwrap_or_default());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `needle` and every failure, in that order.
fn with_failures(needle: &str) -> Vec<&str> {
    let mut all: Vec<&str> = needle.split(" | ").collect();
    all.extend_from_slice(FAILURES);
    all
}

/// Run `cmd` and require `needle`, or one of its alternatives between
/// ` | `: the catalogue's text and what QS/400 sends where the two
/// differ (`docs/QS400_GUEST.md`); the screen.
fn ok(
    guest: &mut Session,
    id: u32,
    cmd: &str,
    needle: &str,
    within: Duration,
) -> Result<String, String> {
    let successes = needle.split(" | ").count();
    match command(guest, id, cmd, &with_failures(needle), within)? {
        (i, s) if i < successes => Ok(s),
        (_, s) => Err(format!("`{cmd}` failed:\n{s}")),
    }
}

/// Run `cmd` and say which of `needles` or the failures showed, with
/// its line.
fn outcome(guest: &mut Session, id: u32, cmd: &str, needles: &[&str]) -> Result<String, String> {
    let mut all: Vec<&str> = needles.to_vec();
    all.extend(FAILURES.iter().filter(|f| !needles.contains(f)));
    let (i, s) = command(guest, id, cmd, &all, Duration::from_mins(10))?;
    Ok(format!("`{}`", line_with(&s, all[i])))
}

/// A display command: the first screen it shows, then F3 until Command
/// Entry is back. A message instead of a display is the error.
fn display(guest: &mut Session, id: u32, cmd: &str) -> Result<String, String> {
    let m = mark(guest, id)?;
    enter(guest, id, cmd)?;
    let deadline = Instant::now() + Duration::from_mins(5);
    let shown_screen = loop {
        let s = screen(guest, id)?.text;
        if !s.contains(COMMAND_ENTRY) {
            // Let the panel finish drawing.
            std::thread::sleep(Duration::from_secs(2));
            break screen(guest, id)?.text;
        }
        if let Some(i) = shown(&s, &m, FAILURES) {
            return Err(format!("`{cmd}`: {}", line_with(&s, FAILURES[i])));
        }
        if Instant::now() >= deadline {
            return Err(format!("`{cmd}` showed nothing:\n{s}"));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    for _ in 0..8 {
        keys(guest, id, &[key(Key::Function(3))])?;
        let back = Instant::now() + Duration::from_secs(15);
        if wait_screen(guest, id, &[COMMAND_ENTRY], back).is_ok() {
            return Ok(shown_screen);
        }
    }
    Err(format!("F3 did not return to Command Entry after `{cmd}`"))
}

/// The slot's medium as the partition manager has it; empty for none.
fn mounted(guest: &mut Session, id: u32) -> Result<String, String> {
    Ok(said(partd(guest)?.media(id))?
        .into_iter()
        .find(|m| m.slot == SLOT)
        .map(|m| m.name)
        .unwrap_or_default())
}

/// Unmount the slot's volume from the host, waiting while the system
/// prevents its removal; its name.
fn unmount(guest: &mut Session, id: u32) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_mins(2);
    loop {
        match partd(guest)?.unload_media(id, SLOT, false) {
            Ok(Ok(name)) => return Ok(name),
            Ok(Err(e)) => {
                if Instant::now() >= deadline {
                    return Err(manager::refused(&e));
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Mount the volume `name` in the slot from the host.
fn mount(guest: &mut Session, id: u32, name: &str) -> Result<(), String> {
    said(partd(guest)?.load_media(id, SLOT, name.into()))
}

/// The first line of `screen` that has `needle`, trimmed.
fn line_with<'s>(screen: &'s str, needle: &str) -> &'s str {
    screen
        .lines()
        .rfind(|l| l.contains(needle))
        .map_or("", str::trim)
}

/// Each named object and the first word of its row that is a type, in
/// name order.
fn typed_objects(screen: &str, names: &[&str]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = screen
        .lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let name = words.next()?;
            if !names.contains(&name) {
                return None;
            }
            let kind = words.find(|w| w.starts_with('*'))?;
            Some((name.to_string(), kind.to_string()))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The objects of TAPLIB: one of each of several types.
const OBJECTS: &[(&str, &str)] = &[
    (
        "DA1",
        "CRTDTAARA DTAARA(TAPLIB/DA1) TYPE(*CHAR) LEN(20) VALUE('saved to tape')",
    ),
    ("QSRC", "CRTSRCPF FILE(TAPLIB/QSRC) RCDLEN(92)"),
    ("MQ1", "CRTMSGQ MSGQ(TAPLIB/MQ1)"),
    ("DQ1", "CRTDTAQ DTAQ(TAPLIB/DQ1) MAXLEN(64)"),
    ("SF1", "CRTSAVF FILE(TAPLIB/SF1)"),
    ("JD1", "CRTJOBD JOBD(TAPLIB/JD1) TEXT('tape save')"),
];

/// Suite 2, its first part: `INZTAP`, `SAVLIB` of a library of several
/// object types, `DSPTAP`; the volume unmounted, mounted on the second
/// partition's drive and `RSTLIB` there; the objects compared.
pub fn save(run: &Run<'_>) -> Result<String, String> {
    let mut rig = rig(run, &[("SAVE1", "SAV001", 64)])?;
    let (first, second) = (rig.first, rig.second);
    let g = &mut rig.guest;
    start(g, first, profile("/dev/vdb", Some("SAVE1")), run)?;
    let mut notes = String::new();
    let two = Duration::from_mins(2);
    ok(
        g,
        first,
        "CRTLIB LIB(TAPLIB)",
        "Library TAPLIB created | TAPLIB *LIB created",
        two,
    )?;
    for (_, cmd) in OBJECTS {
        ok(g, first, cmd, "reated", two)?;
    }
    let names: Vec<&str> = OBJECTS.iter().map(|(n, _)| *n).collect();
    let before = display(g, first, "DSPOBJD OBJ(TAPLIB/*ALL) OBJTYPE(*ALL)")?;
    let saved = typed_objects(&before, &names);
    if saved.len() != OBJECTS.len() {
        return Err(format!("TAPLIB's objects: {saved:?}\n{before}"));
    }
    ok(
        g,
        first,
        &format!("INZTAP DEV({DEVICE}) NEWVOL(SAV001) CHECK(*NO)"),
        "Volume SAV001 is initialized | Volume SAV001 initialized",
        Duration::from_mins(5),
    )?;
    let s = ok(
        g,
        first,
        &format!("SAVLIB LIB(TAPLIB) DEV({DEVICE})"),
        "objects saved from library TAPLIB",
        Duration::from_mins(15),
    )?;
    let _ = writeln!(notes, "SAVLIB: `{}`", line_with(&s, "objects saved"));
    let labels = display(g, first, &format!("DSPTAP DEV({DEVICE})"))?;
    if !labels.contains("SAV001") {
        return Err(format!("DSPTAP:\n{labels}"));
    }
    let _ = writeln!(notes, "DSPTAP: `{}`", line_with(&labels, "SAV001"));
    // The volume to the second partition, by the host.
    let name = unmount(g, first)?;
    if name != "SAVE1" {
        return Err(format!("unmounted {name}"));
    }
    start(g, second, profile("/dev/vdc", None), run)?;
    mount(g, second, "SAVE1")?;
    let s = ok(
        g,
        second,
        &format!("RSTLIB SAVLIB(TAPLIB) DEV({DEVICE})"),
        "objects restored to library TAPLIB | objects restored to TAPLIB",
        Duration::from_mins(15),
    )?;
    let _ = writeln!(notes, "RSTLIB: `{}`", line_with(&s, "objects restored"));
    let after_restore = display(g, second, "DSPOBJD OBJ(TAPLIB/*ALL) OBJTYPE(*ALL)")?;
    let restored = typed_objects(&after_restore, &names);
    if restored != saved {
        return Err(format!("saved {saved:?}, restored {restored:?}\n{notes}"));
    }
    let value = display(g, second, "DSPDTAARA DTAARA(TAPLIB/DA1)")?;
    if !value.contains("saved to tape") {
        return Err(format!("the data area restored:\n{value}"));
    }
    let kinds: Vec<&str> = saved.iter().map(|(_, k)| k.as_str()).collect();
    Ok(format!(
        "INZTAP CPC6773; SAVLIB of TAPLIB CPC3722; DSPTAP read the volume; unmounted by the \
         host and mounted on SECOND's drive; RSTLIB CPC3703; the objects ({}) and the data \
         area's value the same | {}",
        kinds.join(" "),
        notes.trim_end().replace('\n', " | ")
    ))
}

/// Wait until the system has unloaded the slot's volume (QS/400's
/// `MTOFFL` at the end of a volume), or one of `done` shows after
/// `mark`; whether it was unloaded.
fn until_unloaded(
    guest: &mut Session,
    id: u32,
    mark: &str,
    done: &[&str],
    within: Duration,
) -> Result<bool, String> {
    let deadline = Instant::now() + within;
    loop {
        if mounted(guest, id)?.is_empty() {
            return Ok(true);
        }
        if let Ok(s) = screen(guest, id)
            && shown(&s.text, mark, done).is_some()
        {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            return Err("the volume was never unloaded".into());
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// The reply list's G to the next-volume inquiry, and this job taking
/// its replies from the list.
fn reply_g(guest: &mut Session, id: u32) -> Result<(), String> {
    command(
        guest,
        id,
        "ADDRPYLE SEQNBR(9075) MSGID(CPA4075) RPY(G)",
        &["added", "Added", "exists", "already", "reply"],
        Duration::from_mins(2),
    )?;
    let m = mark(guest, id)?;
    enter(guest, id, "CHGJOB INQMSGRPY(*SYSRPYL)")?;
    // CHGJOB may answer nothing: its echo, then a moment.
    until(guest, id, &m, &["*SYSRPYL"], Duration::from_mins(1))
        .map_err(|s| format!("CHGJOB:\n{s}"))?;
    std::thread::sleep(Duration::from_secs(3));
    Ok(())
}

/// BIGLIB, larger than one volume: 2 Mi records of 10 bytes, made by
/// doubling a stream file in Qshell and copying it into a member.
fn big_library(guest: &mut Session, id: u32, notes: &mut String) -> Result<(), String> {
    ok(
        guest,
        id,
        "CRTLIB LIB(BIGLIB)",
        "Library BIGLIB created | BIGLIB *LIB created",
        Duration::from_mins(2),
    )?;
    let m = mark(guest, id)?;
    enter(
        guest,
        id,
        "QSH CMD('cd /tmp;echo 012345678>b;for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 \
         18 19 20 21;do cat b b>c;mv c b;done;ls -l b')",
    )?;
    let (which, s) = until(
        guest,
        id,
        &m,
        &["Press ENTER", "exit status", "rror"],
        Duration::from_mins(30),
    )
    .map_err(|s| format!("QSH:\n{s}"))?;
    let _ = writeln!(notes, "QSH: `{}`", line_with(&s, " b"));
    if which == 0 {
        keys(guest, id, &[key(Key::Enter)])?;
        wait_screen(
            guest,
            id,
            &[COMMAND_ENTRY],
            Instant::now() + Duration::from_mins(1),
        )?;
    }
    ok(
        guest,
        id,
        "CRTPF FILE(BIGLIB/BIG) RCDLEN(10) SIZE(*NOMAX)",
        "reated",
        Duration::from_mins(2),
    )?;
    let copied = outcome(
        guest,
        id,
        "CPYFRMSTMF FROMSTMF('/tmp/b') TOMBR('/QSYS.LIB/BIGLIB.LIB/BIG.FILE/BIG.MBR') \
         MBROPT(*REPLACE)",
        &["copied", "Copied"],
    )?;
    let _ = writeln!(notes, "CPYFRMSTMF: {copied}");
    Ok(())
}

/// Run `cmd`, which reads or writes SPAN1 then SPAN2: when the system
/// unloads SPAN1 at its end and asks for the next, the host mounts
/// SPAN2. The screen once `success` or a failure shows.
fn over_two(
    guest: &mut Session,
    id: u32,
    cmd: &str,
    success: &str,
    notes: &mut String,
) -> Result<String, String> {
    let began = Instant::now();
    let m = mark(guest, id)?;
    enter(guest, id, cmd)?;
    let done = with_failures(success);
    let hour = Duration::from_mins(60);
    if !until_unloaded(guest, id, &m, &done, hour)? {
        let s = screen(guest, id)?.text;
        return Err(format!("`{cmd}` ended without SPN001's end:\n{s}\n{notes}"));
    }
    let _ = writeln!(
        notes,
        "SPN001 unloaded by the system {:.0} s into `{cmd}`",
        began.elapsed().as_secs_f64()
    );
    mount(guest, id, "SPAN2")?;
    let (which, s) =
        until(guest, id, &m, &done, hour).map_err(|s| format!("`{cmd}`:\n{s}\n{notes}"))?;
    if which != 0 {
        return Err(format!("`{cmd}` over two volumes:\n{s}\n{notes}"));
    }
    Ok(s)
}

/// Suite 2, its second part: a save past a small volume's capacity:
/// the early warning, the end of the volume, the request for the next,
/// a second volume mounted from the host, and the restore across both.
pub fn span(run: &Run<'_>) -> Result<String, String> {
    let mut rig = rig(run, &[("SPAN1", "SPN001", 16), ("SPAN2", "SPN002", 16)])?;
    let (first, second) = (rig.first, rig.second);
    let g = &mut rig.guest;
    start(g, first, profile("/dev/vdb", Some("SPAN2")), run)?;
    let mut notes = String::new();
    let five = Duration::from_mins(5);
    let inz = |v: &str| format!("INZTAP DEV({DEVICE}) NEWVOL({v}) CHECK(*NO)");
    ok(
        g,
        first,
        &inz("SPN002"),
        "Volume SPN002 is initialized | Volume SPN002 initialized",
        five,
    )?;
    unmount(g, first)?;
    mount(g, first, "SPAN1")?;
    ok(
        g,
        first,
        &inz("SPN001"),
        "Volume SPN001 is initialized | Volume SPN001 initialized",
        five,
    )?;
    big_library(g, first, &mut notes)?;
    reply_g(g, first)?;
    let s = over_two(
        g,
        first,
        &format!("SAVLIB LIB(BIGLIB) DEV({DEVICE}) VOL(SPN001 SPN002)"),
        "objects saved from library BIGLIB",
        &mut notes,
    )?;
    let _ = writeln!(notes, "SAVLIB: `{}`", line_with(&s, "objects saved"));
    // The restore on the second partition, across both volumes.
    unmount(g, first)?;
    start(g, second, profile("/dev/vdc", None), run)?;
    mount(g, second, "SPAN1")?;
    reply_g(g, second)?;
    let s = over_two(
        g,
        second,
        &format!("RSTLIB SAVLIB(BIGLIB) DEV({DEVICE}) VOL(SPN001 SPN002)"),
        "objects restored to library BIGLIB | objects restored to BIGLIB",
        &mut notes,
    )?;
    let _ = writeln!(notes, "RSTLIB: `{}`", line_with(&s, "objects restored"));
    let fd = display(g, second, "DSPFD FILE(BIGLIB/BIG) TYPE(*MBR)")?;
    let _ = writeln!(notes, "DSPFD: `{}`", line_with(&fd, "ecords"));
    if !fd.contains("2097152") && !fd.contains("2,097,152") {
        return Err(format!("the restored file:\n{fd}\n{notes}"));
    }
    Ok(notes.trim_end().replace('\n', " | "))
}

/// An AWSTAPE image of `objects`: a block's bytes, or `None` for a
/// filemark.
fn aws(objects: &[Option<Vec<u8>>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut prev = 0u16;
    for o in objects {
        match o {
            None => {
                out.extend_from_slice(&tape::aws::Header::filemark(prev).encode());
                prev = 0;
            }
            Some(block) => {
                let mut at = 0;
                for h in tape::aws::Header::for_block(block.len(), prev) {
                    let len = usize::from(h.len);
                    out.extend_from_slice(&h.encode());
                    out.extend_from_slice(&block[at..at + len]);
                    at += len;
                    prev = h.len;
                }
            }
        }
    }
    out
}

/// An 80-byte ASCII label of `fields`, padded with spaces.
fn label(fields: &[&str]) -> Vec<u8> {
    let mut l: Vec<u8> = fields.concat().into_bytes();
    l.resize(80, b' ');
    l
}

/// A volume as a Unix system writes an ANSI-labelled tape (ISO 1001,
/// ASCII): VOL1, HDR1, HDR2, a file of fixed 80-byte records in 800-byte
/// blocks, EOF1, EOF2.
fn ansi_volume() -> Vec<u8> {
    let vol1 = label(&[
        "VOL1",
        "ANS001",
        " ",
        &" ".repeat(13),
        "UNIX-TAPE    ",
        "OPERATOR      ",
        &" ".repeat(28),
        "4",
    ]);
    let hdr1 = |kind: &str| {
        label(&[
            kind,
            "PAYROLL.DAT      ",
            "ANS001",
            "0001",
            "0001",
            "0001",
            "00",
            " 26321",
            " 00000",
            " ",
            "000000",
            "UNIX-TAPE    ",
        ])
    };
    let hdr2 = |kind: &str| label(&[kind, "F", "00800", "00080", &" ".repeat(35), "00"]);
    let data: Vec<u8> = (0..800u32)
        .map(|i| b'A' + u8::try_from(i % 26).unwrap_or(0))
        .collect();
    aws(&[
        Some(vol1),
        Some(hdr1("HDR1")),
        Some(hdr2("HDR2")),
        None,
        Some(data.clone()),
        Some(data),
        None,
        Some(hdr1("EOF1")),
        Some(hdr2("EOF2")),
        None,
        None,
    ])
}

/// A volume written with no labels, as `tar` writes one: 10240-byte
/// blocks and two filemarks.
fn raw_volume() -> Vec<u8> {
    let mut block = vec![0u8; 10240];
    block[..9].copy_from_slice(b"README.md");
    block[257..262].copy_from_slice(b"ustar");
    aws(&[Some(block), Some(vec![0u8; 10240]), None, None])
}

/// What `DSPTAP` showed of a volume: its type or identifier with its
/// line, or why it showed nothing.
fn dsptap(guest: &mut Session, id: u32, identifier: &str) -> String {
    match display(guest, id, &format!("DSPTAP DEV({DEVICE})")) {
        Ok(s) => {
            let what = ["*BLANK", "*NL", "*SL", identifier]
                .into_iter()
                .filter(|w| s.contains(w))
                .map(|w| format!("`{}`", line_with(&s, w)))
                .collect::<Vec<_>>();
            if what.is_empty() {
                format!("a display without the volume: {}", s.replace('\n', " / "))
            } else {
                what.join(", ")
            }
        }
        Err(e) => e,
    }
}

/// Suite 2, its third part: a write-protected volume, an unlabelled
/// one, and volumes initialised by something else, each through
/// `DSPTAP`, `SAVLIB` and `INZTAP CHECK(*YES)`.
pub fn volumes(run: &Run<'_>) -> Result<String, String> {
    let mut rig = rig(
        run,
        &[
            ("PROT1", "PRT001", 16),
            ("BLANK1", "BLK001", 16),
            ("ANSI1", "ANS001", 16),
            ("RAW1", "RAW001", 16),
        ],
    )?;
    let first = rig.first;
    let mount_point = rig.mount.clone();
    let g = &mut rig.guest;
    for (name, bytes) in [("ANSI1", ansi_volume()), ("RAW1", raw_volume())] {
        let path = format!("{mount_point}/media/tape/{name}.aws");
        channel(g)?.ask(&format!("put {path} {}", hex::encode(&bytes)))?;
    }
    start(g, first, profile("/dev/vdb", Some("PROT1")), run)?;
    let mut notes = String::new();
    // Write-protected: labelled first, then protected at the host.
    ok(
        g,
        first,
        &format!("INZTAP DEV({DEVICE}) NEWVOL(PRT001) CHECK(*NO)"),
        "Volume PRT001 is initialized | Volume PRT001 initialized",
        Duration::from_mins(5),
    )?;
    ok(
        g,
        first,
        "CRTLIB LIB(SMALL)",
        "Library SMALL created | SMALL *LIB created",
        Duration::from_mins(2),
    )?;
    unmount(g, first)?;
    crate::suite::storage::said(poold(g)?.protect_volume("PROT1".into(), true))?;
    mount(g, first, "PROT1")?;
    let save = format!("SAVLIB LIB(SMALL) DEV({DEVICE})");
    let saved = ["objects saved from library SMALL"];
    let o = outcome(g, first, &save, &saved)?;
    let _ = writeln!(notes, "PROT1 protected: SAVLIB {o}");
    let o = outcome(
        g,
        first,
        &format!("INZTAP DEV({DEVICE}) NEWVOL(PRT002) CHECK(*NO)"),
        &["is initialized", "initialized on device"],
    )?;
    let _ = writeln!(notes, "PROT1 protected: INZTAP {o}");
    let _ = writeln!(
        notes,
        "PROT1 protected: DSPTAP {}",
        dsptap(g, first, "PRT001")
    );
    // Unlabelled, then initialised by something else.
    for (name, identifier) in [
        ("BLANK1", "BLK001"),
        ("ANSI1", "ANS001"),
        ("RAW1", "RAW001"),
    ] {
        unmount(g, first)?;
        mount(g, first, name)?;
        let _ = writeln!(notes, "{name}: DSPTAP {}", dsptap(g, first, identifier));
        let o = outcome(g, first, &save, &saved)?;
        let _ = writeln!(notes, "{name}: SAVLIB {o}");
        let o = outcome(
            g,
            first,
            &format!("INZTAP DEV({DEVICE}) NEWVOL({identifier}) CHECK(*YES)"),
            &["is initialized", "initialized on device", "has not expired"],
        )?;
        let _ = writeln!(notes, "{name}: INZTAP CHECK(*YES) {o}");
    }
    Ok(notes.trim_end().replace('\n', " | "))
}
