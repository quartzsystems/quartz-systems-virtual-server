// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 6 to 11 and 15: code update over QSI. The good path with a
//! partition running and undisturbed until the activation shuts it
//! down; a rollback; each refusal; power cut during installs and trial
//! IPLs; a mirrored load source; back to the older release; `qsictl`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ed25519_dalek::Signer;
use qsi::any::{AnyEvent, AnyReply, AnyRequest};
use qsi::host::model::PartitionState;
use qsi::{Empty, update as q};
use qsictl::service::Update;

use super::{
    Console, Opts, Rig, console, host, other_release, signing_key, state_sums, this_release,
};
use crate::suite::base::{Run, channel};
use crate::suite::install::on_disk;
use crate::suite::qsi::eventually;
use crate::suite::qsi::flows::{activate, make_partition, wait_state};

/// How long an activation may take: partitions shut down, the restart,
/// the trial's IPL.
const ACTIVATION: Duration = Duration::from_mins(20);

/// Stage `file` over QSI and wait for its verification.
fn stage(c: &mut Console, file: &Path, allow_older: bool) -> Result<(), String> {
    qsictl::service::update(
        &mut c.link,
        Update::Stage {
            file: file.to_path_buf(),
            allow_older,
        },
    )
}

/// Install what is staged and wait until it is pending.
fn install(c: &mut Console) -> Result<(), String> {
    qsictl::service::update(&mut c.link, Update::Install)
}

/// Activate, shutting partitions down through their systems.
fn activate_update(c: &mut Console, shut_down: bool) -> Result<(), String> {
    qsictl::service::update(
        &mut c.link,
        Update::Activate {
            shut_down,
            timeout: 300,
        },
    )
}

/// The update family's status.
fn status(c: &mut Console) -> Result<q::Status, String> {
    match c.ask(AnyRequest::Update(q::Request::Status(Empty)))? {
        AnyReply::Update(q::Reply::Status(s)) => Ok(s),
        other => Err(format!("{other:?}")),
    }
}

/// The slot the host runs from, and its release.
fn running(c: &mut Console) -> Result<(String, String), String> {
    let s = status(c)?;
    s.slots
        .iter()
        .find(|s| s.running)
        .map(|s| (s.name.clone(), s.release.clone()))
        .ok_or_else(|| format!("no slot runs: {s:?}"))
}

/// The release the image at `file` holds.
fn release_of(file: &Path) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let p = update::image::parse(&bytes).map_err(|d| d.0)?;
    let text = std::str::from_utf8(&bytes[p.manifest]).map_err(|e| e.to_string())?;
    Ok(update::Manifest::parse(text)?.release)
}

/// Stage, install and activate `file`; the host back after its
/// restart, the trial confirmed: the console linked again, its events
/// resumed from before. The Confirmed event's release.
fn update_to<'a>(
    run: &Run<'a>,
    rig: &mut Rig<'a>,
    c: &mut Console,
    file: &Path,
    allow_older: bool,
    shut_down: bool,
) -> Result<String, String> {
    stage(c, file, allow_older)?;
    install(c)?;
    let before = c.subscribe(None)?.next.saturating_sub(1);
    activate_update(c, shut_down)?;
    c.link.close();
    rig.restarted_slow(run)?;
    let deadline = Instant::now() + ACTIVATION;
    rig.q
        .guest
        .seen(&refcode::UPDATE_CONFIRMED.tag(), deadline)?;
    c.again(&rig.q, Duration::from_mins(2))?;
    let resumed = c.subscribe(Some(before))?;
    if resumed.replay != qsi::base::Replay::Resumed {
        return Err(format!("the events after the restart: {resumed:?}"));
    }
    c.until(Duration::from_secs(30), |e| match e {
        AnyEvent::Update(q::Event::Confirmed(cf)) => Some(cf.release.clone()),
        _ => None,
    })
}

impl<'a> Rig<'a> {
    /// The host restarted by itself after an activation, which shuts
    /// partitions down first: started again on the same disks.
    fn restarted_slow(&mut self, run: &Run<'a>) -> Result<(), String> {
        // The restart is the update's: the trial may need its time.
        let launch = self.q.launch.clone();
        self.q.guest.wait_exit(Instant::now() + ACTIVATION)?;
        let mut guest = run.boot(&launch)?;
        guest.seen(&refcode::QSI_STARTED.tag(), Instant::now() + ACTIVATION)?;
        self.q.guest = guest;
        Ok(())
    }
}

/// Every partition's definition and profiles, as QSI reads them.
fn partitions_whole(c: &mut Console) -> Result<String, String> {
    use qsi::host::{partitions, profiles};
    let AnyReply::Partitions(partitions::Reply::List(all)) =
        c.ask(AnyRequest::Partitions(partitions::Request::List(Empty)))?
    else {
        return Err("list".into());
    };
    let mut out = String::new();
    for p in all {
        // Its definition: its state is the activation's to change.
        let AnyReply::Partitions(partitions::Reply::Get(d)) = c.ask(AnyRequest::Partitions(
            partitions::Request::Get(partitions::Get { id: p.id }),
        ))?
        else {
            return Err("get".into());
        };
        let _ = writeln!(out, "{:?}", d.definition);
        let AnyReply::Profiles(profiles::Reply::List(names)) = c.ask(AnyRequest::Profiles(
            profiles::Request::List(profiles::List { id: p.id }),
        ))?
        else {
            return Err("profiles".into());
        };
        for n in names {
            let pr = c.ask(AnyRequest::Profiles(profiles::Request::Get(
                profiles::Get {
                    id: p.id,
                    name: n.name.clone(),
                },
            )))?;
            let _ = writeln!(out, "{pr:?}");
        }
    }
    Ok(out)
}

/// Suite 6, the good path: V1R0M1 staged and installed over QSI with a
/// partition running and undisturbed, activated with the partitions
/// shut down first; the host comes up on the other slot, confirms,
/// reports the new release, and its partitions and pairing are as
/// before.
pub fn good(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let want = release_of(&image)?;
    let mut rig = host(
        run,
        &Opts {
            partition: true,
            ..Opts::default()
        },
    )?;
    let deadline = run.deadline();
    let mut c = console(&mut rig.q, "MC1", deadline)?;
    let (slot0, rel0) = running(&mut c)?;
    let id = make_partition(&mut c.link, "UPD")?;
    activate(&mut c.link, id, None, "")?;
    wait_state(
        &mut c.link,
        id,
        PartitionState::Running,
        Duration::from_mins(15),
    )?;
    let pid = |rig: &mut Rig<'_>| -> Result<u32, String> {
        let mut p = crate::suite::partition::manager::partd(&mut rig.q.guest)?;
        Ok(partd::Calls::list(&mut p)
            .map_err(|e| e.to_string())?
            .iter()
            .find(|s| s.id == id)
            .map_or(0, |s| s.pid))
    };
    let vmm = pid(&mut rig)?;
    let whole = partitions_whole(&mut c)?;
    stage(&mut c, &image, false)?;
    install(&mut c)?;
    // Installed: the partition runs on as it did.
    if pid(&mut rig)? != vmm {
        return Err("the install disturbed the partition's VMM".into());
    }
    let s = status(&mut c)?;
    let pending: Vec<&q::Slot> = s
        .slots
        .iter()
        .filter(|s| s.last == q::Attempt::Pending)
        .collect();
    if pending.len() != 1 || pending[0].release != want || pending[0].running {
        return Err(format!("after the install: {s:?}"));
    }
    let confirmed = {
        let before = c.subscribe(None)?.next.saturating_sub(1);
        activate_update(&mut c, true)?;
        c.link.close();
        rig.restarted_slow(run)?;
        rig.q.guest.seen(
            &refcode::UPDATE_CONFIRMED.tag(),
            Instant::now() + ACTIVATION,
        )?;
        c.again(&rig.q, Duration::from_mins(2))?;
        let r = c.subscribe(Some(before))?;
        if r.replay != qsi::base::Replay::Resumed {
            return Err(format!("the events after the restart: {r:?}"));
        }
        c.until(Duration::from_secs(30), |e| match e {
            AnyEvent::Update(q::Event::Confirmed(cf)) => Some(cf.clone()),
            _ => None,
        })?
    };
    let me = c.identify()?;
    let (slot1, rel1) = running(&mut c)?;
    if me.release != want || rel1 != want || slot1 == slot0 || confirmed.slot != slot1 {
        return Err(format!(
            "after the activation: identify {}, running {slot1} {rel1}, confirmed {confirmed:?}",
            me.release
        ));
    }
    let now = partitions_whole(&mut c)?;
    if now != whole {
        return Err(format!(
            "a partition's definition or profile changed across the update:\n{whole}\nnow\n{now}"
        ));
    }
    c.link.close();
    Ok(format!(
        "{rel0} in slot {slot0} with partition {id} running; {want} staged, verified and \
         installed into slot {slot1} with the partition's VMM ({vmm}) undisturbed; activated \
         with the partitions shut down first; the host came back on slot {slot1}, confirmed \
         ({}, event Confirmed on the console's resumed events), identifies as {}; its partition \
         and the console's pairing as before",
        refcode::UPDATE_CONFIRMED.code,
        me.release
    ))
}

/// Suite 7, rollback: an update whose trial never confirms (the
/// `fail-trial` hook: its services taken as never ready) is restarted
/// by its deadline, and the host comes back on the old slot with its
/// state exactly as it was, says rolled back, has the problem open and
/// a host dump taken.
#[allow(clippy::too_many_lines)] // one scenario, in order
pub fn rollback(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let mut rig = host(
        run,
        &Opts {
            hooks: vec!["fail-trial", "short-trial"],
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let (slot0, rel0) = running(&mut c)?;
    let sums = |rig: &mut Rig<'_>| -> Result<super::Sums, String> {
        // The inventory is written again from the hardware at each IPL.
        Ok(state_sums(&mut rig.q)?
            .into_iter()
            .filter(|(p, _)| !p.ends_with(<state::Inventory as state::Document>::FILE))
            .collect())
    };
    stage(&mut c, &image, false)?;
    install(&mut c)?;
    let before_state = sums(&mut rig)?;
    let before = c.subscribe(None)?.next.saturating_sub(1);
    activate_update(&mut c, false)?;
    c.link.close();
    // The trial: up, never confirmed, restarted by its deadline.
    rig.q.guest.wait_exit(Instant::now() + ACTIVATION)?;
    let launch = rig.q.launch.clone();
    let mut trial = run.boot(&launch)?;
    trial.seen(
        "TEST HOOK: the trial's services are taken as never ready",
        Instant::now() + ACTIVATION,
    )?;
    trial.wait_exit(Instant::now() + Duration::from_mins(5))?;
    drop(trial);
    rig.again(run)?;
    let deadline = run.deadline();
    for code in [
        &refcode::UPDATE_STATE_RESTORED,
        &refcode::SLOT_FELL_BACK,
        &refcode::UPDATE_ROLLED_BACK,
    ] {
        if super::super::qsi::posted(&mut rig.q, code) == 0 {
            return Err(format!("{} was not posted", code.code));
        }
    }
    rig.q
        .guest
        .seen(&refcode::HOST_DUMP_WRITTEN.tag(), deadline)?;
    let after_state = sums(&mut rig)?;
    if after_state != before_state {
        let changed: Vec<&String> = after_state
            .iter()
            .filter(|a| !before_state.contains(a))
            .map(|(p, _)| p)
            .collect();
        return Err(format!("the state is not as it was: {changed:?}"));
    }
    c.again(&rig.q, Duration::from_mins(2))?;
    let (slot1, rel1) = running(&mut c)?;
    if (slot1.as_str(), rel1.as_str()) != (slot0.as_str(), rel0.as_str()) {
        return Err(format!("back on {slot1} {rel1}, not {slot0} {rel0}"));
    }
    c.subscribe(Some(before))?;
    let rolled = c.until(Duration::from_secs(30), |e| match e {
        AnyEvent::Update(q::Event::RolledBack(r)) => Some(r.clone()),
        _ => None,
    })?;
    let open = super::problems_of(
        &mut c,
        refcode::SLOT_FELL_BACK.code,
        1,
        Duration::from_secs(30),
    )?;
    // It names the slot that did not confirm.
    if !open[0]
        .description
        .contains(&format!("slot {} did not confirm", rolled.slot))
    {
        return Err(format!("the problem: {}", open[0].description));
    }
    let dumps = match c.ask(AnyRequest::Base(qsi::base::Request::Dumps(
        qsi::base::ListDumps {},
    )))? {
        AnyReply::Base(qsi::base::Reply::Dumps(d)) => d,
        other => return Err(format!("{other:?}")),
    };
    let dump = dumps
        .iter()
        .find(|d| d.by.contains("did not confirm"))
        .ok_or_else(|| format!("no host dump for the rollback: {dumps:?}"))?;
    let s = status(&mut c)?;
    let other = s.slots.iter().find(|s| !s.running).ok_or("no other slot")?;
    if other.last != q::Attempt::RolledBack {
        return Err(format!("the other slot is {:?}", other.last));
    }
    c.link.close();
    Ok(format!(
        "the trial of slot {} never confirmed and was restarted by its deadline; the host came \
         back on slot {slot1} ({rel1}) with every state file as it was ({} files), posted {}, \
         {} and {}; event RolledBack ({}); problem {} open ({}); host dump {} taken by `{}`",
        rolled.slot,
        before_state.len(),
        refcode::UPDATE_STATE_RESTORED.code,
        refcode::SLOT_FELL_BACK.code,
        refcode::UPDATE_ROLLED_BACK.code,
        rolled.reason,
        open[0].number,
        open[0].description,
        dump.name,
        dump.by
    ))
}

/// The image at `from`, its manifest changed by `change` and signed by
/// `key`.
fn crafted(
    from: &Path,
    key: &ed25519_dalek::SigningKey,
    change: impl FnOnce(&mut update::Manifest),
) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(from).map_err(|e| e.to_string())?;
    let p = update::image::parse(&bytes).map_err(|d| d.0)?;
    let text = std::str::from_utf8(&bytes[p.manifest.clone()]).map_err(|e| e.to_string())?;
    let mut m = update::Manifest::parse(text)?;
    change(&mut m);
    let file = |name: &str| {
        p.files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, r)| bytes[r.clone()].to_vec())
            .unwrap_or_default()
    };
    let (a, b) = (file(update::verify::SLOT_A), file(update::verify::SLOT_B));
    Ok(update::make(&m, &a, &b, |t| key.sign(t).to_bytes()))
}

/// Stage `bytes` written to a scratch file: refused, its identifiers.
fn refused_stage(
    run: &Run<'_>,
    c: &mut Console,
    name: &str,
    bytes: &[u8],
    allow_older: bool,
) -> Result<String, String> {
    let file = run.file(name);
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    match stage(c, &file, allow_older) {
        Ok(()) => Err(format!("{name} was staged, and must not have been")),
        Err(e) => Ok(e),
    }
}

/// Suite 8, refusals: a damaged image, a wrong signature, another
/// product's, an older one without leave, one that cannot read the
/// state; an activation with a partition running and no leave.
pub fn refused(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let key = signing_key(run)?;
    let mut rig = host(
        run,
        &Opts {
            partition: true,
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let mut lines = Vec::new();
    let mut expect = |what: &str, got: String, id: &str| -> Result<(), String> {
        if !got.starts_with(id) {
            return Err(format!("{what}: {got}, not {id}"));
        }
        lines.push(format!("{what} {id}"));
        Ok(())
    };
    let mut damaged = std::fs::read(&image).map_err(|e| e.to_string())?;
    let at = damaged.len() - 4096;
    damaged[at] ^= 0x5A;
    let got = refused_stage(run, &mut c, "damaged.upd", &damaged, false)?;
    expect("a damaged image", got, q::msgid::DAMAGED)?;
    let stranger = ed25519_dalek::SigningKey::from_bytes(&[0x33; 32]);
    let got = refused_stage(
        run,
        &mut c,
        "stranger.upd",
        &crafted(&image, &stranger, |_| {})?,
        false,
    )?;
    expect("a stranger's signature", got, q::msgid::BAD_SIGNATURE)?;
    let other = crafted(&image, &key, |m| m.product = "QS/MC (test image)".into())?;
    let got = refused_stage(run, &mut c, "other.upd", &other, false)?;
    expect("another product's", got, q::msgid::WRONG_PRODUCT)?;
    let older = crafted(&image, &key, |m| m.release = "V0R9M9".into())?;
    let got = refused_stage(run, &mut c, "older.upd", &older, false)?;
    expect("an older one without leave", got, q::msgid::OLDER)?;
    let blind = crafted(&image, &key, |m| {
        for s in &mut m.schemas {
            s.1 = 0;
        }
    })?;
    let got = refused_stage(run, &mut c, "blind.upd", &blind, true)?;
    expect(
        "one that cannot read the state, even with leave",
        got,
        q::msgid::NO_STATE,
    )?;
    // A partition running, and an activation that may not shut it down.
    stage(&mut c, &image, false)?;
    install(&mut c)?;
    let id = make_partition(&mut c.link, "RUN")?;
    activate(&mut c.link, id, None, "")?;
    let ids = c.refused(&AnyRequest::Update(q::Request::Activate(q::Activate {
        partitions: q::Partitions::Refuse,
        timeout: 300,
    })))?;
    if ids.first().map(String::as_str) != Some(q::msgid::PARTITIONS_RUN) {
        return Err(format!("an activation with a partition running: {ids:?}"));
    }
    lines.push(format!(
        "an activation with a partition running {}",
        ids.join(" ")
    ));
    c.ask(AnyRequest::Update(q::Request::Cancel(Empty)))?;
    let s = status(&mut c)?;
    if s.slots.iter().any(|s| s.last == q::Attempt::Pending) {
        return Err(format!("still pending after the cancel: {s:?}"));
    }
    c.link.close();
    Ok(format!("refused: {}; cancelled after", lines.join("; ")))
}

/// Suite 9: power cut at random moments while an update is installed
/// and while a pending slot has its first IPL: the host always comes up
/// on a good slot with its state whole.
#[allow(clippy::too_many_lines)] // one scenario, in order
pub fn power(run: &Run<'_>) -> Result<String, String> {
    let newer = other_release()?;
    let older = this_release(run)?;
    let mut rig = host(run, &Opts::default())?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| u64::from(d.subsec_nanos()) | 1);
    let mut rng = crate::suite::base::power::Moments(seed);
    let mut lines = Vec::new();
    let check = |rig: &mut Rig<'_>, c: &mut Console| -> Result<(String, String), String> {
        c.again(&rig.q, Duration::from_mins(2))?;
        // Every state file reads.
        for (path, _) in state_sums(&mut rig.q)? {
            if std::path::Path::new(&path)
                .extension()
                .is_some_and(|e| e == "toml")
            {
                let text = channel(&mut rig.q.guest)?.text(&path)?;
                toml::from_str::<toml::Table>(&text).map_err(|e| format!("{path}: {e}"))?;
            }
        }
        running(c)
    };
    for i in 0..15 {
        // During an install.
        let (_, rel) = running(&mut c)?;
        let file = if rel == release_of(&newer)? {
            &older
        } else {
            &newer
        };
        let _ = c.ask(AnyRequest::Update(q::Request::Cancel(Empty)));
        let _ = c.ask(AnyRequest::Update(q::Request::Discard(Empty)));
        stage(&mut c, file, true)?;
        c.ask(AnyRequest::Update(q::Request::Install(Empty)))?;
        std::thread::sleep(Duration::from_millis(rng.number() % 4000));
        c.link.close();
        rig.power_cut(run)?;
        let (slot, rel) = check(&mut rig, &mut c)?;
        let s = status(&mut c)?;
        let pending: Vec<String> = s
            .slots
            .iter()
            .filter(|s| s.last == q::Attempt::Pending)
            .map(|s| s.name.clone())
            .collect();
        // Pending only when the slot is whole on the disk.
        if let Some(p) = pending.first() {
            let disk = on_disk(&rig.disks[0])?;
            let uki = if p == "A" { disk.slot_a } else { disk.slot_b };
            let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
            let parsed = update::image::parse(&bytes).map_err(|d| d.0)?;
            let want = if p == "A" {
                update::verify::SLOT_A
            } else {
                update::verify::SLOT_B
            };
            let (_, r) = parsed
                .files
                .iter()
                .find(|(n, _)| n == want)
                .ok_or("no slot in the image")?;
            if uki != bytes[r.clone()] {
                return Err(format!("cut {i}: slot {p} is pending and not whole"));
            }
        }
        lines.push(format!(
            "install cut {i}: up on {slot} {rel}, pending {pending:?}"
        ));
    }
    for i in 0..15 {
        // During a pending slot's first IPL.
        let (_, rel) = running(&mut c)?;
        let file = if rel == release_of(&newer)? {
            &older
        } else {
            &newer
        };
        let _ = c.ask(AnyRequest::Update(q::Request::Cancel(Empty)));
        let _ = c.ask(AnyRequest::Update(q::Request::Discard(Empty)));
        stage(&mut c, file, true)?;
        install(&mut c)?;
        activate_update(&mut c, false)?;
        c.link.close();
        rig.q.guest.wait_exit(Instant::now() + ACTIVATION)?;
        let launch = rig.q.launch.clone();
        let mut trial = run.boot(&launch)?;
        std::thread::sleep(Duration::from_millis(500 + rng.number() % 29_500));
        trial.kill();
        drop(trial);
        rig.again(run)?;
        let (slot, rel) = check(&mut rig, &mut c)?;
        lines.push(format!("trial cut {i}: up on {slot} {rel}"));
    }
    c.link.close();
    Ok(format!(
        "30 power cuts (seed {seed}), the host up on a good slot with whole state each time: {}",
        lines.join("; ")
    ))
}

/// The bytes of slot `slot`'s UKI in the image at `file`.
fn uki_of(file: &Path, slot: &str) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let p = update::image::parse(&bytes).map_err(|d| d.0)?;
    let want = if slot == "A" {
        update::verify::SLOT_A
    } else {
        update::verify::SLOT_B
    };
    p.files
        .iter()
        .find(|(n, _)| n == want)
        .map(|(_, r)| bytes[r.clone()].to_vec())
        .ok_or_else(|| format!("no {want}"))
}

/// Suite 10: on a mirrored load source the update is on both disks, and
/// with either disk removed after the activation the host IPLs the new
/// release.
pub fn mirror(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let want = release_of(&image)?;
    let mut rig = host(
        run,
        &Opts {
            mirrored: true,
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let release = update_to(run, &mut rig, &mut c, &image, false, false)?;
    let (slot, _) = running(&mut c)?;
    c.link.close();
    for d in &rig.disks {
        let disk = on_disk(d)?;
        let uki = if slot == "A" {
            disk.slot_a
        } else {
            disk.slot_b
        };
        if uki != uki_of(&image, &slot)? {
            return Err(format!("{}: slot {slot} is not the update's", d.display()));
        }
    }
    crate::suite::install::power_off(&mut rig.q.guest)?;
    let mut lines = Vec::new();
    for keep in 0..2 {
        let mut launch = rig.q.launch.clone();
        let gone = rig.disks[1 - keep].clone();
        launch.extra.retain(|d| d.path != gone);
        let mut guest = crate::suite::base::booted(run, &launch)?;
        let status = channel(&mut guest)?.status()?;
        let rel = status
            .fact(release::fact::RELEASE)
            .unwrap_or("")
            .to_string();
        if rel != want {
            return Err(format!("with disk {} alone the host runs {rel}", keep + 1));
        }
        lines.push(format!("disk {} alone: {rel}", keep + 1));
        crate::suite::install::power_off(&mut guest)?;
    }
    Ok(format!(
        "mirrored: {release} confirmed in slot {slot}, the slot's UKI the update's on both disks; {}",
        lines.join(", ")
    ))
}

/// Suite 11: from the newer release, an older image without leave is
/// refused, and with leave installs and confirms.
pub fn back(run: &Run<'_>) -> Result<String, String> {
    let newer = other_release()?;
    let older = this_release(run)?;
    let mut rig = host(run, &Opts::default())?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let up = update_to(run, &mut rig, &mut c, &newer, false, false)?;
    let refused = match stage(&mut c, &older, false) {
        Ok(()) => return Err("the older image was staged without leave".into()),
        Err(e) => e,
    };
    if !refused.starts_with(q::msgid::OLDER) {
        return Err(format!("the older image without leave: {refused}"));
    }
    let down = update_to(run, &mut rig, &mut c, &older, true, false)?;
    let (slot, rel) = running(&mut c)?;
    c.link.close();
    if rel != down {
        return Err(format!("running {rel}, confirmed {down}"));
    }
    Ok(format!(
        "up to {up}; the older {down} refused without leave ({}), then with leave installed and \
         confirmed in slot {slot}",
        refused.split_whitespace().next().unwrap_or("")
    ))
}

/// Suite 15: `qsictl` runs the update's good path, and lists and closes
/// a problem.
pub fn qsictl(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let mut rig = host(run, &Opts::default())?;
    let deadline = run.deadline();
    crate::suite::qsi::to_pairing(&mut rig.q, deadline)?;
    let (password, _) = crate::suite::qsi::start_pairing(&mut rig.q, deadline)?;
    let dir: PathBuf = run.file("qsictl");
    let bin = crate::suite::qsi::flows::qsictl_binary()?;
    let run_it = |args: &[&str]| -> Result<String, String> {
        let out = std::process::Command::new(&bin)
            .arg("--dir")
            .arg(&dir)
            .args(args)
            .output()
            .map_err(|e| format!("qsictl: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!(
                "qsictl {args:?}: {}{text}",
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    };
    run_it(&["init"])?;
    run_it(&["pair", &rig.q.address, &password, "--as", "vs"])?;
    // Leave the pairing panel only once it is used: leaving ends it.
    super::sign_off(&mut rig.q.guest)?;
    let staged = run_it(&["update", "stage", &image.display().to_string()])?;
    let installed = run_it(&["update", "install"])?;
    run_it(&["update", "activate"])?;
    rig.restarted_slow(run)?;
    rig.q.guest.seen(
        &refcode::UPDATE_CONFIRMED.tag(),
        Instant::now() + ACTIVATION,
    )?;
    let now = eventually(Duration::from_mins(2), || {
        run_it(&["update", "status"]).ok()
    })
    .ok_or("qsictl update status after the restart")?;
    let want = release_of(&image)?;
    if !now.contains(&want) {
        return Err(format!("after the restart:\n{now}"));
    }
    // A code that opens a problem, posted through the hook (a failing
    // service would keep the trial from confirming).
    let code = &refcode::CLOCK_NOT_SET;
    crate::suite::base::channel(&mut rig.q.guest)?.ask(&format!(
        "{} {} by the harness",
        qsvshook::proto::command::POST,
        code.code
    ))?;
    let listed = eventually(Duration::from_mins(2), || {
        run_it(&["problems", "list", "--open"])
            .ok()
            .filter(|t| t.contains(code.code))
    })
    .ok_or_else(|| format!("qsictl lists no open problem of {}", code.code))?;
    let number = listed
        .lines()
        .find(|l| l.contains(code.code))
        .and_then(|l| l.split_whitespace().next())
        .ok_or("no number")?
        .to_string();
    let closed = run_it(&["problems", "close", &number, "--anyway"])?;
    let shown = run_it(&["problems", "show", &number])?;
    if !shown.contains("Closed") {
        return Err(format!("problem {number} after its close:\n{shown}"));
    }
    Ok(format!(
        "qsictl staged ({}), installed ({}), activated; the host confirmed {want}; it listed \
         problem {number} and closed it ({})",
        staged.trim(),
        installed.lines().last().unwrap_or("").trim(),
        closed.trim()
    ))
}

/// The update image of a test release whose partition definitions are
/// of schema 2 (`QSVS_TEST_SCHEMA=1`, `docs/PHASE11.md`), signed by the
/// test image's key.
pub const SCHEMA_ENV: &str = "QSVS_UPDATE_SCHEMA_IMAGE";
/// Keys a test image may trust beside its own (`QSVS_TRUSTED_KEYS`):
/// the second, which it trusts, and a third, which it does not.
pub const KEY2_ENV: &str = "QSVS_UPDATE_KEY2";
/// See [`KEY2_ENV`].
pub const KEY3_ENV: &str = "QSVS_UPDATE_KEY3";

fn env_file(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("{name} names no file (docs/PHASE11.md, Section 6)"))
}

fn key_file(name: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let path = env_file(name)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let seed: [u8; 32] = hex::decode(text.trim())
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| format!("{name}: a key of 32 bytes"))?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&seed))
}

/// The schema version the file at `path` on the host is written in.
fn schema_of(rig: &mut Rig<'_>, path: &str) -> Result<u32, String> {
    let text = channel(&mut rig.q.guest)?.text(path)?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("schema_version = ")?.parse().ok())
        .ok_or_else(|| format!("{path} names no schema version"))
}

/// Section 6's migration: a release that changes the partition
/// definition's schema. A trial that never confirms leaves the old form;
/// confirmed, the definition is written in the new form when it next
/// changes; then the older release, which cannot read that form, is
/// refused even with leave to go back.
pub fn migrate(run: &Run<'_>) -> Result<String, String> {
    let newer = env_file(SCHEMA_ENV)?;
    let older = this_release(run)?;
    let mut rig = host(
        run,
        &Opts {
            hooks: vec!["fail-trial", "short-trial"],
            ..Opts::default()
        },
    )?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let id = make_partition(&mut c.link, "KEEP")?;
    let path = format!("/data/state/partitions/{id}/definition.toml");
    let first = schema_of(&mut rig, &path)?;

    // A trial that never confirms: rolled back, the old form kept.
    stage(&mut c, &newer, false)?;
    install(&mut c)?;
    activate_update(&mut c, false)?;
    c.link.close();
    rig.q.guest.wait_exit(Instant::now() + ACTIVATION)?;
    let launch = rig.q.launch.clone();
    let mut trial = run.boot(&launch)?;
    trial.seen(
        "TEST HOOK: the trial's services are taken as never ready",
        Instant::now() + ACTIVATION,
    )?;
    trial.wait_exit(Instant::now() + Duration::from_mins(5))?;
    drop(trial);
    rig.again(run)?;
    if super::super::qsi::posted(&mut rig.q, &refcode::UPDATE_ROLLED_BACK) == 0 {
        return Err("the trial was not rolled back".into());
    }
    let rolled = schema_of(&mut rig, &path)?;

    // Without the hooks: confirmed, and the changed definition written
    // in the new form.
    rig.q.guest.kill();
    rig.q.launch.oem.clear();
    rig.again(run)?;
    c.again(&rig.q, Duration::from_mins(2))?;
    let confirmed = update_to(run, &mut rig, &mut c, &newer, false, false)?;
    let unchanged = schema_of(&mut rig, &path)?;
    {
        use qsi::host::{partition, partitions};
        let AnyReply::Partitions(partitions::Reply::Get(p)) = c.ask(AnyRequest::Partitions(
            partitions::Request::Get(partitions::Get { id }),
        ))?
        else {
            return Err("the definition was not read".into());
        };
        let mut changed = p.definition.clone();
        changed.name = "KEPT".into();
        c.ask(AnyRequest::Partition(partition::Request::Change(
            partition::Change {
                definition: changed,
            },
        )))?;
    }
    let written = schema_of(&mut rig, &path)?;

    // The older release cannot read it: refused, leave or not.
    let back = match stage(&mut c, &older, true) {
        Ok(()) => return Err("the older release was staged over the newer form".into()),
        Err(e) => e,
    };
    if !back.starts_with(q::msgid::NO_STATE) {
        return Err(format!("going back: {back}, not {}", q::msgid::NO_STATE));
    }
    // This release's definitions are of its version; the test release
    // reads the one after it.
    let v = <model::Definition as state::Document>::VERSION;
    if (first, rolled, unchanged, written) != (v, v, v, v + 1) {
        return Err(format!(
            "the definition's schema: {first} before, {rolled} after the rollback, {unchanged} \
             once confirmed, {written} once changed"
        ));
    }
    Ok(format!(
        "definition.toml: schema {v}; a trial of the release that reads {next}, never \
         confirmed, rolled back with the file still {v}; {confirmed} confirmed, the file {v} \
         until the definition changed, then {next}; the older release refused even with \
         leave: `{back}`",
        next = v + 1
    ))
}

/// Section 6's keys: a host that trusts a second key beside its own
/// takes a release signed by it, whole, and refuses one signed by a key
/// it does not trust.
pub fn keys(run: &Run<'_>) -> Result<String, String> {
    let image = other_release()?;
    let second = key_file(KEY2_ENV)?;
    let third = key_file(KEY3_ENV)?;
    let mut rig = host(run, &Opts::default())?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    let by_third = crafted(&image, &third, |_| {})?;
    let refused = refused_stage(run, &mut c, "by-third.upd", &by_third, false)?;
    if !refused.starts_with(q::msgid::BAD_SIGNATURE) {
        return Err(format!("signed by a key not trusted: {refused}"));
    }
    let by_second = run.file("by-second.upd");
    std::fs::write(&by_second, crafted(&image, &second, |_| {})?).map_err(|e| e.to_string())?;
    let confirmed = update_to(run, &mut rig, &mut c, &by_second, false, false)?;
    Ok(format!(
        "signed by the second key ({}), which the host trusts beside its own: staged, installed, \
         {confirmed} confirmed; signed by a third ({}): `{refused}`",
        hex::encode(second.verifying_key().to_bytes()),
        hex::encode(third.verifying_key().to_bytes())
    ))
}
