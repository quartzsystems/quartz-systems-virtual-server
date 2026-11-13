// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/400 in a partition: an unmodified disk image, built by QS/400's
//! own orchestrator at the pinned commit, on a block device of the
//! host. An IPL is complete where QS/400's own conformance takes it
//! to be: `SRC C6001090`. The guest is spoken to through its service
//! console, on its serial port.

use std::time::{Duration, Instant};

use super::{
    NAME, QS400_MEMORY_MIB, UNIT, ended, expect_code, expect_guest, expect_started, fresh_host,
    guest_line, host, host_unaffected, launch, profile, qs400_image, send, signal, start, unit,
};
use crate::qemu::session::Session;
use crate::suite::base::{Guest, Run};

/// The codes of a normal IPL of QS/400, in the order its own
/// conformance wants them. Others may come between.
pub const IPL_CODES: [&str; 16] = [
    "C1001000", "C1001010", "C1001020", "C1001030", "C1001070", "C1001080", "C1001040", "C1001060",
    "C6001000", "C6001005", "C6001010", "C6001012", "C6001030", "C6001040", "C6001050", "C6001090",
];
/// The last of them.
pub const IPL_COMPLETE: &str = "SRC C6001090";
/// The code after which the guest's disk is being written.
const UNITS_LOCATED: &str = "SRC C1001070";
/// What the guest says when its service console takes commands.
pub const CONSOLE_READY: &str = "Service console ready";
/// The last code of a power down.
const POWER_DOWN_COMPLETE: &str = "SRC D1001090";
/// The codes that end an IPL without completing it.
const FATAL: [&str; 3] = ["SRC B1", "SRC C1001085", "SRC C1001087"];

/// How long an IPL of QS/400 may take in a partition of a host that
/// is itself a guest.
pub fn patience(run: &Run<'_>) -> Instant {
    Instant::now() + 6 * run.timeout
}

/// Boot a host with a copy of the QS/400 image as the partition's
/// load source; the hook starts the partition.
pub fn qs400<'a>(run: &Run<'a>, processors: u32, ipl: &str) -> Result<Guest<'a>, String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let disk = unit(run, &image)?;
    let text = profile(processors, QS400_MEMORY_MIB, ipl, UNIT);
    host(run, &launch(run, &[disk], &text)?)
}

/// The guest's IPL codes in order, to "IPL complete".
pub fn ipl_complete(guest: &mut Session, deadline: Instant) -> Result<(), String> {
    for code in IPL_CODES {
        if let Err(e) = expect_guest(guest, &format!("SRC {code}"), deadline) {
            return Err(format!("{e}; {}", super::diagnosis(guest)));
        }
    }
    for fatal in FATAL {
        if guest.said(&guest_line(fatal)) {
            return Err(format!("the guest posted a code starting {fatal}"));
        }
    }
    Ok(())
}

/// Wait until the guest's service console takes commands.
pub fn console_ready(guest: &mut Session, deadline: Instant) -> Result<(), String> {
    expect_guest(guest, CONSOLE_READY, deadline)
}

/// Have the guest power down, or restart, and wait for `qsvmm` to end.
fn power_down(guest: &mut Session, restart: bool, deadline: Instant) -> Result<String, String> {
    send(guest, if restart { "pwrdwn restart" } else { "pwrdwn" })?;
    expect_guest(guest, POWER_DOWN_COMPLETE, deadline)?;
    ended(guest, deadline)
}

fn ipl_from(run: &Run<'_>, side: char, processors: u32) -> Result<String, String> {
    let lower = side.to_ascii_lowercase();
    let mut guest = qs400(run, processors, &format!("side = \"{lower}\""))?;
    let deadline = patience(run);
    let began = Instant::now();
    let started = expect_started(&mut guest, deadline)?;
    if !started.contains(&format!("side {side}")) {
        return Err(format!("side {side} asked for: `{started}`"));
    }
    ipl_complete(&mut guest, deadline)?;
    let took = began.elapsed().as_secs_f64();
    // The side the guest itself says it runs from: its baked command line.
    guest.seen(&guest_line(&format!("slic: slot {side}")), deadline)?;
    console_ready(&mut guest, deadline)?;
    // Every processor of the profile is one of the guest's resources.
    send(&mut guest, "hw")?;
    for n in 1..=processors {
        expect_guest(&mut guest, &format!("PN{n:02} "), deadline)?;
    }
    guest.expect("1af4:1048 virtio SCSI", deadline)?;
    let how = power_down(&mut guest, false, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_POWERED_OFF, deadline)?;
    if how != "exit 0" {
        return Err(format!("the guest powered off; qsvmm gave `{how}`"));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "QS/400 IPLed from side {side} with {processors} processors: the {} codes in order to \
         C6001090 in {took:.1} s from the activation; the guest names side {side}, its \
         processors PN01 to PN{processors:02} and the virtio SCSI controller; powered off, exit 0",
        IPL_CODES.len()
    ))
}

/// Suite 3: QS/400 IPLs from side A.
pub fn ipl_a(run: &Run<'_>) -> Result<String, String> {
    ipl_from(run, 'A', 2)
}

/// Suite 4: the same from side B, and with four processors.
pub fn ipl_b(run: &Run<'_>) -> Result<String, String> {
    ipl_from(run, 'B', 4)
}

/// One activation with the profile's side, to the guest's service
/// console: the side `qsvmm` loaded and what it read in the record.
fn activation(
    run: &Run<'_>,
    guest: &mut Session,
    side: Option<&str>,
) -> Result<(char, String), String> {
    if let Some(side) = side {
        let ipl = format!("side = \"{side}\"");
        start(guest, Some(&profile(2, QS400_MEMORY_MIB, &ipl, UNIT)))?;
    }
    let deadline = patience(run);
    let started = expect_started(guest, deadline)?;
    let mark = crate::suite::base::posted(&refcode::PARTITION_RECORD_READ, "");
    let read = guest
        .log
        .rfind(&mark)
        .and_then(|at| guest.log[at + mark.len()..].lines().next())
        .unwrap_or("")
        .trim()
        .to_string();
    let loaded = ['A', 'B']
        .into_iter()
        .find(|s| started.contains(&format!("side {s}")))
        .ok_or_else(|| format!("no side in `{started}`"))?;
    ipl_complete(guest, deadline)?;
    // The side the guest itself says it runs from.
    guest.seen(&guest_line(&format!("slic: slot {loaded}")), deadline)?;
    console_ready(guest, deadline)?;
    Ok((loaded, read))
}

/// Power the guest down with `command`, typed at its service console.
fn power_down_with(run: &Run<'_>, guest: &mut Session, command: &str) -> Result<(), String> {
    let deadline = patience(run);
    send(guest, command)?;
    expect_guest(guest, POWER_DOWN_COMPLETE, deadline)?;
    let how = ended(guest, deadline)?;
    expect_code(guest, &refcode::PARTITION_POWERED_OFF, deadline)?;
    if how == "exit 0" {
        Ok(())
    } else {
        Err(format!("`{command}`: qsvmm gave `{how}`"))
    }
}

/// Step 0's suite: `side = "recorded"` is what the partition itself
/// asked for. `IPLSRC(B)` at a power down in the guest IPLs side B
/// once; an explicit side overrides the request and leaves it.
pub fn recorded(run: &Run<'_>) -> Result<String, String> {
    const ASK_B: &str = "pwrdwnsys OPTION(*IMMED) IPLSRC(B)";
    let armed = guest_line("slic: next IPL from slot B");
    // The side loaded, and what the record asked for as it was read.
    let want = |n: u32, got: &(char, String), side: char, asked: &str| {
        if got.0 == side && got.1.starts_with(&format!("{asked} keylock")) {
            Ok(())
        } else {
            Err(format!(
                "IPL {n}: side {side} wanted with the record asking `{asked}`: side {} 
                 loaded, the record read as `{}`",
                got.0, got.1
            ))
        }
    };

    // Nothing asked yet, and no record to ask through: a disk that
    // has never IPLed. Side A.
    let mut guest = qs400(run, 2, "side = \"recorded\"")?;
    let first = activation(run, &mut guest, None)?;
    if first.0 != 'A' || !first.1.is_empty() {
        return Err(format!(
            "IPL 1, of a disk that has never IPLed: side {} loaded, the record read as `{}`",
            first.0, first.1
        ));
    }
    power_down_with(run, &mut guest, ASK_B)?;
    if !guest.said(&armed) {
        return Err("the guest did not say `next IPL from slot B`".into());
    }

    // The request is taken: side B, and the record written without it.
    let second = activation(run, &mut guest, Some("recorded"))?;
    want(2, &second, 'B', "side B once")?;
    if !guest.said(&crate::suite::base::posted(
        &refcode::PARTITION_RECORD_WRITTEN,
        "",
    )) {
        return Err("the once-only side was not cleared in the record".into());
    }
    power_down_with(run, &mut guest, "pwrdwn")?;

    // It was for one IPL only.
    let third = activation(run, &mut guest, Some("recorded"))?;
    want(3, &third, 'A', "side A")?;
    power_down_with(run, &mut guest, ASK_B)?;

    // An explicit side overrides the request and leaves it standing.
    let fourth = activation(run, &mut guest, Some("a"))?;
    want(4, &fourth, 'A', "side B once")?;
    power_down_with(run, &mut guest, "pwrdwn")?;
    let fifth = activation(run, &mut guest, Some("recorded"))?;
    want(5, &fifth, 'B', "side B once")?;
    power_down_with(run, &mut guest, "pwrdwn")?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "recorded with nothing asked: side A; `{ASK_B}` in the guest, then recorded: `{}`, side \
         B, the guest names slot B; the IPL after it: `{}`, side A; asked again, then an \
         activation with side a: side A and the request left (`{}`); the next recorded \
         activation: side B",
        second.1, third.1, fourth.1
    ))
}

/// Press Enter on the guest's console, through its service console.
fn enter(guest: &mut Session) -> Result<(), String> {
    send(guest, "key enter")
}

/// Suite 5: the keylock position and the console mode the host writes
/// are the ones the guest reports.
pub fn attributes(run: &Run<'_>) -> Result<String, String> {
    let manual = "side = \"a\"\nkeylock = \"manual\"\nconsole = \"serial\"";
    let mut guest = qs400(run, 2, manual)?;
    let deadline = patience(run);
    expect_code(&mut guest, &refcode::PARTITION_RECORD_WRITTEN, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    // The guest reads its record: attended, and its console on serial.
    expect_guest(&mut guest, "slic: mode Manual from LIC override", deadline)?;
    guest.expect("(Serial console (ttyS0))", deadline)?;
    expect_guest(&mut guest, "SRC C1001050", deadline)?;
    // An attended IPL stops at its menu, drawn as text on the serial port.
    guest.expect("IPL or Install the System", deadline)?;
    guest.expect(" Selection", deadline)?;
    if guest.said(&guest_line(super::qs400::IPL_COMPLETE)) {
        return Err("an attended IPL completed without its menu being answered".into());
    }
    std::thread::sleep(Duration::from_secs(1));
    send(&mut guest, "type 1")?;
    enter(&mut guest)?;
    guest.expect("IPL Options", deadline)?;
    std::thread::sleep(Duration::from_secs(1));
    enter(&mut guest)?;
    guest.expect("Set Major System Options", deadline)?;
    std::thread::sleep(Duration::from_secs(1));
    enter(&mut guest)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    std::thread::sleep(Duration::from_secs(3));
    send(&mut guest, "lic")?;
    expect_guest(&mut guest, "last_ipl_mode   Manual", deadline)?;
    power_down(&mut guest, false, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_POWERED_OFF, deadline)?;

    // Back to unattended and the display: the guest says so too.
    let normal = "side = \"a\"\nkeylock = \"normal\"\nconsole = \"local\"";
    start(
        &mut guest,
        Some(&profile(2, QS400_MEMORY_MIB, normal, UNIT)),
    )?;
    let deadline = patience(run);
    let read = expect_code(&mut guest, &refcode::PARTITION_RECORD_READ, deadline)?;
    if !read.contains("keylock *NORMAL console local") {
        return Err(format!("the record after the second write: `{read}`"));
    }
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    guest.expect("(Local console (display and keyboard))", deadline)?;
    // A partition has no display: the guest goes on on its serial port.
    expect_guest(&mut guest, "SRC A6005008", deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    send(&mut guest, "lic")?;
    expect_guest(&mut guest, "last_ipl_mode   Normal", deadline)?;
    if guest.log.matches(&guest_line("slic: mode Manual")).count() != 1 {
        return Err("the second IPL was attended too".into());
    }
    power_down(&mut guest, false, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "keylock manual and console serial, written by the host: {NAME} says `mode Manual from \
         LIC override` and `Serial console (ttyS0)`, stops at its IPL menu on the serial port \
         and completes when answered; then normal and local: an unattended IPL, `Local console`, \
         A6005008 for the display a partition has not"
    ))
}

/// Suite 6: `qsvmm` killed during the guest's IPL and again while the
/// guest runs. The host stays up, each end is posted with its signal,
/// and the guest's disk IPLs afterwards.
pub fn kill(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400(run, 2, "side = \"a\"")?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, UNITS_LOCATED, deadline)?;
    signal(&mut guest, "KILL")?;
    let first = expect_code(&mut guest, &refcode::PARTITION_KILLED, deadline)?;
    if !first.contains("signal 9") || ended(&mut guest, deadline)? != "signal 9" {
        return Err(format!("killed during the IPL: `{first}`"));
    }
    host_unaffected(&mut guest)?;

    // The IPL after it completes, and says the one before did not.
    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    ipl_complete(&mut guest, deadline)?;
    let noticed = guest.said(&guest_line("SRC C1002000"));
    console_ready(&mut guest, deadline)?;
    send(
        &mut guest,
        "CRTDTAARA DTAARA(QGPL/KILLED) TYPE(*CHAR) LEN(50)",
    )?;
    // An answer follows the console's prompt or starts a line.
    guest.expect("Data area KILLED created.", deadline)?;
    signal(&mut guest, "KILL")?;
    let second = expect_code(&mut guest, &refcode::PARTITION_KILLED, deadline)?;
    if !second.contains("signal 9") {
        return Err(format!("killed while it ran: `{second}`"));
    }
    host_unaffected(&mut guest)?;

    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    ipl_complete(&mut guest, deadline)?;
    console_ready(&mut guest, deadline)?;
    // What the guest had acknowledged before the kill is there.
    send(&mut guest, "CHKOBJ OBJ(QGPL/KILLED) OBJTYPE(*DTAARA)")?;
    guest.expect("QGPL/KILLED *DTAARA exists.", deadline)?;
    let how = power_down(&mut guest, false, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "killed during the IPL and again while it ran: A2005150 with signal 9 both times, the \
         host's base ready throughout; the IPL after each kill completed{}, an object created \
         before the second kill is there after it; then powered off, {how}",
        if noticed {
            " and noted the abnormal end (C1002000)"
        } else {
            ""
        }
    ))
}

/// Suite 8: a guest's power off and a guest's restart each end
/// `qsvmm` with their own code; a restart is not done in place.
pub fn end(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400(run, 2, "side = \"a\"")?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    let off = power_down(&mut guest, false, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_POWERED_OFF, deadline)?;
    if off != "exit 0" {
        return Err(format!("the guest powered off; qsvmm gave `{off}`"));
    }

    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    let activations = |guest: &mut Session| {
        guest.drain();
        guest
            .log
            .matches(refcode::PARTITION_ACTIVATING.code)
            .count()
    };
    let before = activations(&mut guest);
    let restart = power_down(&mut guest, true, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_RESTART, deadline)?;
    if restart != "exit 10" {
        return Err(format!("the guest restarted; qsvmm gave `{restart}`"));
    }
    // Nothing starts it again: that is whoever started it's to decide.
    std::thread::sleep(Duration::from_secs(5));
    if activations(&mut guest) != before {
        return Err("the partition was started again by itself".into());
    }

    // And an end the host asks for.
    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, UNITS_LOCATED, deadline)?;
    signal(&mut guest, "TERM")?;
    expect_code(&mut guest, &refcode::PARTITION_ENDED, deadline)?;
    let asked = ended(&mut guest, deadline)?;
    if asked != "exit 50" {
        return Err(format!("ended by the host; qsvmm gave `{asked}`"));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "power off: C2005100, {off}; restart: C2005110, {restart}, and no activation followed \
         by itself; SIGTERM from the host: A2005140, {asked}"
    ))
}
