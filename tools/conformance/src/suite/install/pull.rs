// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 6 and 7: the power pulled at random moments, during an
//! installation and on a running mirrored host. An installation cut
//! off anywhere is started over and ends good; a mirrored host cut off
//! anywhere mounts `/data`, resynchronizes, and tears no state file.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use release::fact;

use super::{
    HOST, Kind, SERIALS, answers, blank_disks, boot, install_unattended, installed, machine,
    on_disk, power_off, protected, with_media, with_seed,
};
use crate::suite::base::power::{Moments, verify};
use crate::suite::base::{Run, channel, fact_of, posted};

/// How often the power is pulled.
const CUTS: u32 = 20;
/// An installation is cut off within this long of QEMU's start: the
/// firmware, the kernel, the verification and every step fall in it.
const INSTALL_MS: u64 = 16_000;
/// A running host is cut off within this long of its services' start.
const RUN_ON_MS: u64 = 4000;
/// What a resynchronization is given.
const SYNC_WITHIN: Duration = Duration::from_secs(240);

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| u64::from(d.subsec_nanos()) | 1)
}

/// The steps the progress display marks done.
fn steps_done(text: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().starts_with("X  C2004"))
        .count()
}

/// Suite 6.
pub fn install(run: &Run<'_>) -> Result<String, String> {
    let disks = blank_disks(run, "pull", 2)?;
    let paths: Vec<&Path> = disks.iter().map(PathBuf::as_path).collect();
    let places: Vec<Option<&Path>> = paths.iter().map(|d| Some(*d)).collect();
    let seed_toml = answers(&SERIALS, "", "");
    let launch = with_seed(
        run,
        with_media(run, machine(run, &places), false)?,
        &seed_toml,
    )?;
    let seed = seed();
    let mut moments = Moments(seed);
    // How many pulls came with this many steps done, 0 to 8.
    let mut at_step = [0u32; 9];
    let mut before_it = 0;
    let mut after_it = 0;
    for cut in 0..CUTS {
        let mut guest = run.boot(&launch)?;
        let wait = Duration::from_millis(1500 + moments.number() % INSTALL_MS);
        let ended = guest.wait_exit(Instant::now() + wait).is_ok();
        let shown = guest.grid().text();
        let at = |e: &str| format!("after {cut} power cuts: {e}:\n{shown}");
        // Whatever the cut before left on the disks, the installer
        // starts over: it never stops on what it finds.
        for stopped in ["Installation failed", "is refused", "cannot be used"] {
            if shown.contains(stopped) {
                return Err(at(&format!("the installer says `{stopped}`")));
            }
        }
        if ended {
            // It ran to its end before the moment came.
            after_it += 1;
        } else if shown.contains("Installing.") {
            at_step[steps_done(&shown).min(8)] += 1;
        } else {
            before_it += 1;
        }
        guest.kill();
    }

    // The installation that is let finish, and the host it made.
    install_unattended(run, &paths, &seed_toml, false)?;
    let mut guest = boot(run, &machine(run, &places))?;
    let status = channel(&mut guest)?.status()?;
    if fact_of(&status, fact::HOST)? != HOST {
        return Err(format!("the host is {}", fact_of(&status, fact::HOST)?));
    }
    protected(&mut guest, SYNC_WITHIN)?;
    power_off(&mut guest)?;
    drop(guest);
    for (i, disk) in paths.iter().enumerate() {
        let found = on_disk(disk)?;
        if found.slot_a.is_empty() || found.slot_b.is_empty() {
            return Err(format!("disk {i} lacks a slot's UKI"));
        }
    }
    let steps: Vec<String> = at_step
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(step, n)| format!("{n} with {step} step(s) done"))
        .collect();
    Ok(format!(
        "{CUTS} power cuts during a mirrored installation (seed {seed}): {before_it} before the \
         first step, {}, {after_it} after the last; the installer started over each time; the \
         installation then let finish IPLs, is {HOST}, and its two units came in step",
        steps.join(", ")
    ))
}

/// Suite 7.
pub fn mirror(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Mirrored)?;
    let places: Vec<Option<&Path>> = disks.iter().map(|d| Some(d.as_path())).collect();
    let mut launch = machine(run, &places);
    launch.oem = vec![qsvshook::oem_string(qsvshook::CHURN)];
    let seed = seed();
    let mut moments = Moments(seed);
    let mut resyncs = 0;
    let mut generations: Vec<u64> = Vec::new();
    for cut in 0..=CUTS {
        let mut guest = run.boot(&launch)?;
        let deadline = run.deadline();
        let at = |e: String| format!("after {cut} power cuts: {e}");
        guest
            .expect(&posted(&refcode::SERVICES_STARTED, ""), deadline)
            .map_err(at)?;
        let generation = verify(guest.channel(deadline).map_err(at)?).map_err(at)?;
        generations.extend(generation);
        for code in [
            &refcode::STATE_INVALID,
            &refcode::DATA_NOT_MOUNTED,
            &refcode::SET_NOT_ASSEMBLED,
            &refcode::UNIT_FAILED,
        ] {
            if guest.said(&code.tag()) {
                return Err(at(format!("the IPL posted {}", code.line())));
            }
        }
        let status = channel(&mut guest).map_err(at)?.status().map_err(at)?;
        let data = fact_of(&status, fact::DATA).map_err(at)?;
        if !data.starts_with("/dev/ublkb") {
            return Err(at(format!("/data is on {data}")));
        }
        // A host that was cut off brings its second unit back in step.
        if cut > 0 {
            guest
                .seen(&posted(&refcode::RESYNC_STARTED, ""), deadline)
                .map_err(at)?;
            resyncs += 1;
        }
        if cut < CUTS {
            std::thread::sleep(Duration::from_millis(moments.number() % RUN_ON_MS));
            guest.kill();
            continue;
        }
        // The last IPL is let finish: the resynchronization completes.
        protected(&mut guest, SYNC_WITHIN).map_err(at)?;
        guest
            .seen(&posted(&refcode::RESYNC_FINISHED, ""), run.deadline())
            .map_err(at)?;
        verify(channel(&mut guest).map_err(at)?).map_err(at)?;
        power_off(&mut guest).map_err(at)?;
    }
    for (i, disk) in disks.iter().enumerate() {
        let label = on_disk(disk)?.label.ok_or("a disk lost its set label")?;
        if !label.clean || !label.protected() {
            return Err(format!("disk {i}: the label is {label:?} after it all"));
        }
    }
    let (first, last) = (
        generations.first().copied().unwrap_or(0),
        generations.last().copied().unwrap_or(0),
    );
    if last <= first {
        return Err(format!("the churn did not advance: generation {last}"));
    }
    Ok(format!(
        "{CUTS} power cuts on a running mirrored host (seed {seed}): /data mounted from the \
         protected set at every IPL, a resynchronization started after each of the {resyncs} \
         and the last ran to its end; every state file whole each time (host.toml generation \
         {first} to {last}); powered off clean with both units active"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_done_are_counted_from_the_progress_display() {
        let shown = "  X  C2004020  Partitioning the disks\n  X  C2004030  Setting up protection\n  >  C2004040  Making the data file system\n     C2004050  Copying slot A\n";
        assert_eq!(steps_done(shown), 2);
        assert_eq!(steps_done("Verifying the installation media."), 0);
        assert!(super::super::INSTALL_WITHIN > Duration::from_millis(INSTALL_MS));
    }
}
