// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/400 in a partition: an unmodified disk image, built by QS/400's
//! own orchestrator at the pinned commit, on a block device of the
//! host. An IPL is complete where QS/400's own conformance takes it
//! to be: `SRC C6001090`. The guest is spoken to through its service
//! console, on its serial port.

use std::time::{Duration, Instant};

use partd::{Calls as _, State};

use super::manager::{self, partd, said};
use super::{
    ID, NAME, QS400_MEMORY_MIB, UNIT, ended, expect_code, expect_guest, expect_started, fresh_host,
    guest_line, host_unaffected, hosted, launch, profile, qs400_image, send, signal, start, unit,
};
use crate::qemu::session::Session;
use crate::suite::base::{Guest, Run, posted};

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
/// load source, and activate the partition.
pub fn qs400<'a>(run: &Run<'a>, processors: u32, ipl: &str) -> Result<Guest<'a>, String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let disk = unit(run, &image)?;
    let spec = profile(processors, QS400_MEMORY_MIB, ipl, UNIT);
    hosted(run, &launch(run, &[disk])?, &spec)
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

/// Have the guest power down and wait for `qsvmm` to end.
fn power_down(guest: &mut Session, deadline: Instant) -> Result<String, String> {
    send(guest, "pwrdwn")?;
    expect_guest(guest, POWER_DOWN_COMPLETE, deadline)?;
    ended(guest, deadline)
}

fn ipl_from(run: &Run<'_>, side: char, processors: u32) -> Result<String, String> {
    let lower = side.to_ascii_lowercase();
    let mut guest = qs400(run, processors, &format!("side {lower}"))?;
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
    let how = power_down(&mut guest, deadline)?;
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

/// QS/400 IPLs from side A.
pub fn ipl_a(run: &Run<'_>) -> Result<String, String> {
    ipl_from(run, 'A', 2)
}

/// The same from side B, and with four processors.
pub fn ipl_b(run: &Run<'_>) -> Result<String, String> {
    ipl_from(run, 'B', 4)
}

/// One activation with `ipl` when it is given, to the guest's service
/// console: the side `qsvmm` loaded and what it read in the record.
fn activation(
    run: &Run<'_>,
    guest: &mut Session,
    ipl: Option<&str>,
) -> Result<(char, String), String> {
    if let Some(ipl) = ipl {
        start(guest, Some(&profile(2, QS400_MEMORY_MIB, ipl, UNIT)))?;
    }
    let deadline = patience(run);
    let started = expect_started(guest, deadline)?;
    let mark = posted(&refcode::PARTITION_RECORD_READ, "");
    let read = guest
        .log
        .rfind(&mark)
        .and_then(|at| guest.log[at + mark.len()..].lines().next())
        .map_or("", super::detail_of)
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

/// Step 0's suite: the recorded side is what the partition itself
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
                "IPL {n}: side {side} wanted with the record asking `{asked}`: side {} \
                 loaded, the record read as `{}`",
                got.0, got.1
            ))
        }
    };

    // Nothing asked yet: side A. The keylock the host writes makes the
    // record, on a disk that has never IPLed.
    let mut guest = qs400(run, 2, "side recorded")?;
    let first = activation(run, &mut guest, None)?;
    want(1, &first, 'A', "side A")?;
    power_down_with(run, &mut guest, ASK_B)?;
    if !guest.said(&armed) {
        return Err("the guest did not say `next IPL from slot B`".into());
    }

    // The request is taken: side B, and the record written without it.
    let second = activation(run, &mut guest, Some("side recorded"))?;
    want(2, &second, 'B', "side B once")?;
    power_down_with(run, &mut guest, "pwrdwn")?;

    // It was for one IPL only.
    let third = activation(run, &mut guest, Some("side recorded"))?;
    want(3, &third, 'A', "side A")?;
    power_down_with(run, &mut guest, ASK_B)?;

    // An explicit side overrides the request and leaves it standing.
    let fourth = activation(run, &mut guest, Some("side a"))?;
    want(4, &fourth, 'A', "side B once")?;
    power_down_with(run, &mut guest, "pwrdwn")?;
    let fifth = activation(run, &mut guest, Some("side recorded"))?;
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

/// The keylock position and the console mode the host writes are the
/// ones the guest reports; and the keylock is the host's: what the
/// guest set for itself lasts until the next activation.
pub fn attributes(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400(run, 2, "side a keylock manual console serial")?;
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
    if guest.said(&guest_line(IPL_COMPLETE)) {
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
    power_down(&mut guest, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_POWERED_OFF, deadline)?;

    // Back to unattended and the display: the guest says so too.
    start(
        &mut guest,
        Some(&profile(
            2,
            QS400_MEMORY_MIB,
            "side a keylock normal console local",
            UNIT,
        )),
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

    // The guest asks for a manual IPL itself; the host's keylock, in
    // the normal position, is written at the next activation.
    send(&mut guest, "mode manual")?;
    std::thread::sleep(Duration::from_secs(2));
    power_down(&mut guest, deadline)?;
    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_code(&mut guest, &refcode::PARTITION_STARTED, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    if guest.log.matches(&guest_line("slic: mode Manual")).count() != 1 {
        return Err("the guest's own `mode manual` outlasted an activation".into());
    }
    power_down(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "keylock manual and console serial, written by the host: {NAME} says `mode Manual from \
         LIC override` and `Serial console (ttyS0)`, stops at its IPL menu on the serial port \
         and completes when answered; then normal and local: an unattended IPL, `Local console`, \
         A6005008 for the display a partition has not; then `mode manual` in the guest, and the \
         next activation is unattended: the keylock is the host's"
    ))
}

/// `qsvmm` killed during the guest's IPL and again while the guest
/// runs. The partition goes to Error with its code, the host stays
/// up, and the partition activates again: its disk IPLs.
pub fn kill(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400(run, 2, "side a")?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, UNITS_LOCATED, deadline)?;
    signal(&mut guest, "KILL")?;
    let first = expect_code(&mut guest, &refcode::PARTITION_KILLED, deadline)?;
    if !first.contains("signal 9") || ended(&mut guest, deadline)? != "signal 9" {
        return Err(format!("killed during the IPL: `{first}`"));
    }
    let status = manager::status(&mut guest, ID)?;
    let message = status.message.map(|m| m.to_string()).unwrap_or_default();
    if status.state != State::Error
        || status.code != refcode::PARTITION_KILLED.code
        || status.current.is_some()
        || !message.contains("A2005150")
    {
        return Err(format!(
            "after the kill the partition is {} with `{}`, holding {:?}: {message}",
            status.state.word(),
            status.code,
            status.current
        ));
    }
    host_unaffected(&mut guest)?;
    // Nothing starts it again by itself.
    std::thread::sleep(Duration::from_secs(3));
    if manager::status(&mut guest, ID)?.state != State::Error {
        return Err("a partition in error was started again by itself".into());
    }

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
    ended(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;

    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    ipl_complete(&mut guest, deadline)?;
    console_ready(&mut guest, deadline)?;
    // What the guest had acknowledged before the kill is there.
    send(&mut guest, "CHKOBJ OBJ(QGPL/KILLED) OBJTYPE(*DTAARA)")?;
    guest.expect("QGPL/KILLED *DTAARA exists.", deadline)?;
    let how = power_down(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "killed during the IPL and again while it ran: A2005150 with signal 9 both times, the \
         partition in Error holding nothing ({message}), not started again by itself, the \
         host's base ready throughout; the IPL after each kill completed{}, an object created \
         before the second kill is there after it; then powered off, {how}",
        if noticed {
            " and noted the abnormal end (C1002000)"
        } else {
            ""
        }
    ))
}

/// How many activations the host has posted.
fn activations(guest: &mut Session) -> usize {
    guest.drain();
    guest
        .log
        .matches(&posted(&refcode::PARTITION_ACTIVATING, ""))
        .count()
}

/// The lifecycle: a guest's power off, a guest's restart, and the
/// partition manager's immediate shut down and restart each end in
/// the documented state.
pub fn end(run: &Run<'_>) -> Result<String, String> {
    let mut guest = qs400(run, 2, "side a")?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    manager::until(&mut guest, ID, State::Running, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    let held = manager::status(&mut guest, ID)?.current;

    // The guest powers off: Not activated, holding nothing.
    let off = power_down(&mut guest, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_POWERED_OFF, deadline)?;
    let status = manager::status(&mut guest, ID)?;
    if off != "exit 0" || status.state != State::NotActivated || status.current.is_some() {
        return Err(format!(
            "the guest powered off: qsvmm gave `{off}`, the partition is {} holding {:?}",
            status.state.word(),
            status.current
        ));
    }

    // The guest asks for a restart: activated again at once, with
    // what it held, from the side it asked for.
    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    let before = activations(&mut guest);
    send(&mut guest, "pwrdwn restart")?;
    expect_guest(&mut guest, POWER_DOWN_COMPLETE, deadline)?;
    expect_code(&mut guest, &refcode::PARTITION_RESTART, deadline)?;
    let again = expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    let status = manager::status(&mut guest, ID)?;
    if activations(&mut guest) != before + 1
        || status.state != State::Running
        || status.current != held
    {
        return Err(format!(
            "after the guest's restart the partition is {} holding {:?}, {} activation(s) on",
            status.state.word(),
            status.current,
            activations(&mut guest) - before
        ));
    }

    // Shut down at once by the partition manager.
    said(partd(&mut guest)?.shut_down(ID))?;
    expect_code(&mut guest, &refcode::PARTITION_ENDED, deadline)?;
    let asked = ended(&mut guest, deadline)?;
    let status = manager::status(&mut guest, ID)?;
    if asked != "exit 50" || status.state != State::NotActivated {
        return Err(format!(
            "shut down by the partition manager: qsvmm gave `{asked}`, the partition is {}",
            status.state.word()
        ));
    }
    let refused = partd(&mut guest)?
        .shut_down(ID)
        .map_err(|e| e.to_string())?
        .err()
        .map(|errors| manager::refused(&errors))
        .unwrap_or_default();
    if !refused.starts_with(qvsmsg::PARTITION_NOT_ACTIVE.id) {
        return Err(format!(
            "a shut down of a partition that is not activated answered `{refused}`"
        ));
    }

    // Restarted at once by the partition manager: ended, and
    // activated anew with the profile it had.
    start(&mut guest, None)?;
    let deadline = patience(run);
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, UNITS_LOCATED, deadline)?;
    let before = activations(&mut guest);
    said(partd(&mut guest)?.restart(ID))?;
    expect_code(&mut guest, &refcode::PARTITION_ENDED, deadline)?;
    expect_started(&mut guest, deadline)?;
    expect_guest(&mut guest, IPL_COMPLETE, deadline)?;
    console_ready(&mut guest, deadline)?;
    if activations(&mut guest) != before + 1
        || manager::status(&mut guest, ID)?.state != State::Running
    {
        return Err("a restart by the partition manager did not activate it again once".into());
    }
    power_down(&mut guest, deadline)?;
    host_unaffected(&mut guest)?;
    Ok(format!(
        "guest power off: C2005100, {off}, Not activated holding nothing; guest restart: \
         C2005110, activated again at once with what it held (`{again}`), Running; immediate \
         shut down: A2005140, {asked}, Not activated, and a second one refused ({refused}); \
         immediate restart: ended and activated again, Running"
    ))
}

/// Identity: each partition reports its own serial number, the same
/// at every activation, and QS/400 names its hypervisor.
pub fn identity(run: &Run<'_>) -> Result<String, String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let first = unit(run, &image)?;
    let second = run.file("unit2.img");
    crate::qemu::image::sparse_copy(&image, &second)?;
    let mut guest = super::host(run, &launch(run, &[first, second])?)?;
    let mut seen = Vec::new();
    for (name, device) in [(NAME, UNIT), ("OTHER", "/dev/vdc")] {
        let id = manager::partition(&mut guest, name)?;
        let serial = said(partd(&mut guest)?.definition(id))?.serial;
        let spec = profile(2, QS400_MEMORY_MIB, "side a", device);
        // `slic` says it as it loads its system values, in a line of
        // its own partition.
        let said_serial = format!("QSRLNBR {serial},");
        for activation in 1..=2usize {
            manager::activate(&mut guest, id, &spec).map_err(|errors| manager::refused(&errors))?;
            let deadline = patience(run);
            let at = |e: String| format!("{name}, activation {activation}: {e}");
            guest
                .expect(&super::relayed(name, "SRC C6001010"), deadline)
                .map_err(at)?;
            // Once for each activation so far: the same every time.
            guest.drain();
            let times = guest.log.matches(&said_serial).count();
            if times != activation {
                return Err(at(format!(
                    "the guest said `QSRLNBR {serial}` {times} time(s) in {activation} \
                     activation(s)"
                )));
            }
            guest
                .expect(&super::relayed(name, IPL_COMPLETE), deadline)
                .map_err(at)?;
            guest
                .expect(&super::relayed(name, CONSOLE_READY), deadline)
                .map_err(at)?;
            if activation == 1 {
                super::send_to(&mut guest, id, "qshwqual")?;
                guest
                    .expect("Quartz Systems QS/VS partition", deadline)
                    .map_err(at)?;
                guest
                    .expect("Hypervisor . . . . . :   QS/VS", deadline)
                    .map_err(at)?;
                guest
                    .expect("No firmware: a partition of QS/VS", deadline)
                    .map_err(at)?;
            }
            std::thread::sleep(Duration::from_secs(1));
            super::send_to(&mut guest, id, "pwrdwn")?;
            manager::ended(&mut guest, id, deadline).map_err(at)?;
        }
        seen.push(format!("{name} (partition {id}) {serial}"));
    }
    let serials: Vec<&str> = seen.iter().filter_map(|s| s.rsplit(' ').next()).collect();
    if serials[0] == serials[1] || serials[0][..5] != serials[1][..5] {
        return Err(format!(
            "two partitions of one host have the serial numbers {serials:?}"
        ));
    }
    host_unaffected(&mut guest)?;
    Ok(format!(
        "{}: each guest says its own `QSRLNBR` at two activations; QSHWQUAL names the machine \
         `Quartz Systems QS/VS partition`, the hypervisor `QS/VS`, and passes the firmware \
         check as a partition of QS/VS",
        seen.join(", ")
    ))
}
