// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's power pulled while the guest writes. QEMU is killed, as
//! a pulled plug, at a random moment of the guest creating objects;
//! the host's memory goes with it, and with it anything `qsvmm` had
//! taken from the guest and not yet given to the host's disk. At every
//! IPL after, the guest's object store must recover, and every object
//! the guest had acknowledged before a cut must be there: the guest
//! acknowledges after its flush, so one that is missing was a flush
//! `qsvmm` answered without carrying it out.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::qs400::{console_ready, ipl_complete, patience};
use super::{
    QS400_MEMORY_MIB, UNIT, expect_guest, fresh_host, guest_line, host, launch, profile,
    qs400_image, send, unit,
};
use crate::qemu::session::Session;
use crate::suite::base::Run;
use crate::suite::base::power::Moments;

/// Power cuts.
const CUTS: u32 = 20;
/// The longest the guest is left writing before a cut, in milliseconds.
const WRITE_FOR_MS: u64 = 6000;
/// Objects checked at an IPL: the newest acknowledged ones, which are
/// the ones a lost flush would take.
const CHECKED: usize = 6;

fn name(n: u32) -> String {
    format!("PULL{n:04}")
}

/// Check that the object numbered `n` exists.
fn exists(guest: &mut Session, n: u32, deadline: Instant) -> Result<(), String> {
    let object = name(n);
    send(
        guest,
        &format!("CHKOBJ OBJ(QGPL/{object}) OBJTYPE(*DTAARA)"),
    )?;
    // An answer follows the console's prompt or starts a line.
    let found = format!("QGPL/{object} *DTAARA exists.");
    let lost = format!("CPF9801 object {object}");
    match guest.expect_any(&[&found, &lost], deadline)? {
        0 => Ok(()),
        _ => Err(format!(
            "{object} was acknowledged before a power cut and is gone after it"
        )),
    }
}

/// Create objects until `until`; the numbers the guest acknowledged.
fn write(guest: &mut Session, next: &mut u32, until: Instant) -> Result<Vec<u32>, String> {
    let mut acknowledged = Vec::new();
    while Instant::now() < until {
        let n = *next;
        *next += 1;
        let object = name(n);
        send(
            guest,
            &format!("CRTDTAARA DTAARA(QGPL/{object}) TYPE(*CHAR) LEN(2000)"),
        )?;
        // The cut may come before the guest answers: that object was
        // not acknowledged and may or may not be there.
        let created = format!("Data area {object} created.");
        if guest.expect(&created, until).is_ok() {
            acknowledged.push(n);
        }
    }
    Ok(acknowledged)
}

/// Suite 7: the host's power pulled while the guest writes, twenty
/// times; the guest's object store recovers at each IPL.
pub fn power_pull(run: &Run<'_>) -> Result<String, String> {
    let image = qs400_image()?;
    fresh_host(run)?;
    let disk = unit(run, &image)?;
    let text = profile(2, QS400_MEMORY_MIB, "side = \"a\"", UNIT);
    let l = launch(run, &[disk], &text)?;
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| u64::from(d.subsec_nanos()) | 1);
    let mut moments = Moments(seed);
    let mut acknowledged: Vec<u32> = Vec::new();
    let mut next = 1u32;
    let (mut replayed, mut abnormal) = (0u32, 0u32);
    for cut in 0..=CUTS {
        let at = |e: String| format!("after {cut} power cuts: {e}");
        let mut guest = host(run, &l).map_err(at)?;
        let deadline = patience(run);
        ipl_complete(&mut guest, deadline).map_err(at)?;
        if guest.said(&guest_line("SRC C6001008")) {
            return Err(at("the guest asks for a reclaim of storage".into()));
        }
        abnormal += u32::from(guest.said(&guest_line("SRC C1002000")));
        replayed += u32::from(
            guest.log.contains("transaction(s) replayed")
                && !guest.log.contains(&guest_line(
                    "slic: storage management recovery: 0 transaction(s)",
                )),
        );
        console_ready(&mut guest, deadline).map_err(at)?;
        // The newest acknowledged objects, and the first of all.
        let newest = acknowledged.len().saturating_sub(CHECKED);
        for n in acknowledged
            .first()
            .into_iter()
            .chain(&acknowledged[newest..])
        {
            exists(&mut guest, *n, deadline).map_err(at)?;
        }
        if cut == CUTS {
            // The last IPL: every object ever acknowledged.
            for n in &acknowledged {
                exists(&mut guest, *n, deadline).map_err(at)?;
            }
            send(&mut guest, "pwrdwn").map_err(at)?;
            expect_guest(&mut guest, "SRC D1001090", deadline).map_err(at)?;
            break;
        }
        let until = Instant::now() + Duration::from_millis(500 + moments.number() % WRITE_FOR_MS);
        acknowledged.extend(write(&mut guest, &mut next, until).map_err(at)?);
        // The plug is pulled.
        guest.kill();
    }
    if acknowledged.len() < CUTS as usize {
        return Err(format!(
            "the guest acknowledged only {} objects in {CUTS} runs: it was hardly writing",
            acknowledged.len()
        ));
    }
    Ok(format!(
        "{CUTS} power cuts (seed {seed}) while the guest created objects: the object store \
         recovered at each of {} IPLs ({abnormal} noted as after an abnormal end, {replayed} \
         replayed transactions), and all {} objects the guest acknowledged before a cut are \
         there; {} more were being created when the power went",
        CUTS + 1,
        acknowledged.len(),
        (next - 1) as usize - acknowledged.len()
    ))
}
