// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The install, the activation and the cancel. The install writes the
//! image's UKI for the slot that does not run to every load-source
//! disk's ESP (QS/400's `fat::write_file`: a new file beside the old,
//! renamed over it, so the slot that runs is never written), reads it
//! back, compares, and only then asks PID 1 to mark the slot pending.
//! A power loss before that leaves nothing pending.

use std::fs::{self, OpenOptions};
use std::time::{Duration, Instant};

use partd::Calls as _;
use qvsmsg::Issued;
use supervise::control::Calls as _;

use super::{Held, Shared, lock, now};
use crate::proto::{Attempt, Event, ImageInfo, Progress, Verification};
use crate::verify;

/// Write `data` as the file `path` on the ESP `esp`, synchronized, and
/// read it back: whether it is the same.
fn write_slot(esp: &std::path::Path, path: &str, data: &[u8]) -> Result<(), String> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(esp)
        .map_err(|e| format!("{}: {e}", esp.display()))?;
    diskimg::fat::write_file(&mut f, path, data).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    let f = fs::File::open(esp).map_err(|e| format!("{}: {e}", esp.display()))?;
    let back = diskimg::fat::read_file_at(f, path).map_err(|e| e.to_string())?;
    if back != data {
        return Err("what was read back is not what was written".into());
    }
    Ok(())
}

/// Install the staged, verified image into the slot that does not run:
/// the slot. The writing is a thread's; progress follows as events.
pub(crate) fn install(core: &Shared) -> Result<String, Vec<Issued>> {
    let mut c = lock(core);
    if let Some(b) = &c.busy {
        return Err(vec![qvsmsg::UPDATE_BUSY.with(&[b])]);
    }
    let staged = c
        .staged
        .clone()
        .ok_or_else(|| vec![qvsmsg::UPDATE_NOTHING_STAGED.issue()])?;
    if staged.verification != Verification::Verified {
        let why = staged
            .error
            .first()
            .map_or("it is not verified yet", |i| i.text.as_str());
        return Err(vec![qvsmsg::UPDATE_NOT_VERIFIED.with(&[&staged.name, why])]);
    }
    let slots = c
        .pid1()
        .slots()
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e.to_string()])])?;
    if !slots.pending.is_empty() {
        return Err(vec![qvsmsg::UPDATE_PENDING.with(&[&slots.pending])]);
    }
    let target = c
        .other()
        .ok_or_else(|| vec![qvsmsg::UPDATE_RECORD.with(&["this IPL has no A/B record"])])?;
    c.busy = Some(format!("being installed into slot {target}"));
    c.installing = Some(Progress {
        slot: target.clone(),
        percent: 0,
        time: now(),
    });
    let about = [refcode::about::slot(&target)];
    c.post(
        &refcode::UPDATE_INSTALLING,
        &format!("slot {target}"),
        &about,
    );
    drop(c);
    let core2 = core.clone();
    let slot = target.clone();
    std::thread::spawn(move || {
        let outcome = write_all(&core2, &slot);
        let mut c = lock(&core2);
        c.busy = None;
        c.installing = None;
        let about = [refcode::about::slot(&slot)];
        match outcome {
            Ok(image) => {
                let mut kept = c.kept();
                *kept.of(&slot) = Held {
                    release: image.release.clone(),
                    build: image.build.clone(),
                    fixes: image.fixes.clone(),
                    last: Attempt::Pending,
                    reason: String::new(),
                    time: now(),
                };
                c.keep(&kept);
                c.post(
                    &refcode::UPDATE_INSTALLED,
                    &format!("{} build {} in slot {slot}", image.release, image.build),
                    &about,
                );
                c.publish(&Event::Installed {
                    slot,
                    image: Some(image),
                    error: Vec::new(),
                    time: now(),
                });
            }
            Err(error) => {
                // What the slot holds is not known now.
                let mut kept = c.kept();
                *kept.of(&slot) = Held {
                    reason: "an install did not finish".into(),
                    time: now(),
                    ..Held::default()
                };
                c.keep(&kept);
                let why = error.first().map(|i| i.text.clone()).unwrap_or_default();
                c.post(&refcode::UPDATE_INSTALL_FAILED, &why, &about);
                c.publish(&Event::Installed {
                    slot,
                    image: None,
                    error,
                    time: now(),
                });
            }
        }
    });
    Ok(target)
}

fn progress(core: &Shared, slot: &str, percent: u8) {
    let mut c = lock(core);
    let p = Progress {
        slot: slot.to_string(),
        percent,
        time: now(),
    };
    c.installing = Some(p.clone());
    c.publish(&Event::Installing { progress: p });
}

/// The writing: the image read and verified again, its slot's UKI
/// written to each disk and read back, then the record.
fn write_all(core: &Shared, slot: &str) -> Result<ImageInfo, Vec<Issued>> {
    let (path, running, names, allow_older) = {
        let c = lock(core);
        (
            c.path(super::IMAGE),
            c.running(),
            c.host.names,
            c.allow_older,
        )
    };
    let fail = |why: String| vec![qvsmsg::UPDATE_NOT_WRITTEN.with(&[slot, &why])];
    let bytes = fs::read(&path).map_err(|e| fail(e.to_string()))?;
    let v = verify::verify(&bytes, &running, allow_older).map_err(|r| fail(r.to_string()))?;
    let (file, uki_path) = if slot == "A" {
        (&bytes[v.slots[0].clone()], loadsource::layout::SLOT_A_PATH)
    } else {
        (&bytes[v.slots[1].clone()], names.slot_b_path)
    };
    let sources = loadsource::disks::load_sources(names);
    if sources.is_empty() {
        return Err(fail("no load-source disk was found".into()));
    }
    let n = sources.len();
    for (i, s) in sources.iter().enumerate() {
        write_slot(&s.esp, uki_path, file).map_err(|e| fail(format!("{}: {e}", s.disk)))?;
        let percent = u8::try_from((i + 1) * 90 / n).unwrap_or(90);
        progress(core, slot, percent);
        let c = lock(core);
        c.post(
            &refcode::UPDATE_SLOT_WRITTEN,
            &format!("slot {slot} on {}", s.disk),
            &[refcode::about::slot(slot)],
        );
    }
    let c = lock(core);
    c.pid1()
        .install_slot(slot.to_string())
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e.to_string()])])?
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e])])?;
    drop(c);
    progress(core, slot, 100);
    Ok(ImageInfo {
        product: v.manifest.product,
        release: v.manifest.release,
        build: v.manifest.build,
        fixes: v.manifest.fixes,
        bytes: bytes.len() as u64,
    })
}

/// Restart into the pending slot; partitions running refuse it unless
/// `shut_down`, which shuts each down through its system, each given
/// `timeout` seconds, the rest ended at once after.
pub(crate) fn activate(
    core: &Shared,
    shut_down: bool,
    timeout: u32,
    by: &str,
) -> Result<(), Vec<Issued>> {
    let mut c = lock(core);
    if let Some(b) = &c.busy {
        return Err(vec![qvsmsg::UPDATE_BUSY.with(&[b])]);
    }
    let slots = c
        .pid1()
        .slots()
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e.to_string()])])?;
    if slots.pending.is_empty() {
        return Err(vec![qvsmsg::UPDATE_NOTHING_PENDING.issue()]);
    }
    if slots.activated {
        return Err(vec![qvsmsg::UPDATE_BUSY.with(&["being activated"])]);
    }
    let mut partd = partd::Client::new(&c.host.partd);
    let running: Vec<partd::Status> = partd
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.state.active())
        .collect();
    if !running.is_empty() && !shut_down {
        let names: Vec<String> = running.iter().map(|p| p.name.clone()).collect();
        return Err(vec![
            qvsmsg::UPDATE_PARTITIONS_RUN.with(&[&names.join(", ")]),
        ]);
    }
    let slot = slots.pending.clone();
    c.busy = Some(format!("activating slot {slot}"));
    drop(c);
    let core2 = core.clone();
    let by = by.to_string();
    std::thread::spawn(move || {
        let ids: Vec<u32> = running.iter().map(|p| p.id).collect();
        end_partitions(&mut partd, &ids, timeout);
        let mut c = lock(&core2);
        c.publish(&Event::Activating {
            slot: slot.clone(),
            time: now(),
        });
        let asked = c.pid1().activate_slot(by.clone());
        if !matches!(asked, Ok(Ok(()))) {
            let why = match asked {
                Ok(Err(e)) => e,
                Err(e) => e.to_string(),
                Ok(Ok(())) => String::new(),
            };
            eprintln!("the activation of slot {slot}: {why}");
            c.busy = None;
        }
    });
    Ok(())
}

/// Shut each of `ids` down through its system, then end at once each
/// still active after `timeout` seconds.
fn end_partitions(partd: &mut partd::Client, ids: &[u32], timeout: u32) {
    if ids.is_empty() {
        return;
    }
    for id in ids {
        let _ = partd.shut_down_as(*id, partd::proto::Shutdown::System, timeout, false);
    }
    let active = |p: &mut partd::Client| -> Vec<u32> {
        p.list()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.state.active() && ids.contains(&s.id))
            .map(|s| s.id)
            .collect()
    };
    let until = Instant::now() + Duration::from_secs(u64::from(timeout));
    while Instant::now() < until && !active(partd).is_empty() {
        std::thread::sleep(Duration::from_millis(500));
    }
    for id in active(partd) {
        let _ = partd.shut_down(id);
    }
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until && !active(partd).is_empty() {
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The pending slot is pending no more.
pub(crate) fn cancel(core: &Shared) -> Result<(), Vec<Issued>> {
    let c = lock(core);
    if let Some(b) = &c.busy {
        return Err(vec![qvsmsg::UPDATE_BUSY.with(&[b])]);
    }
    let slots = c
        .pid1()
        .slots()
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e.to_string()])])?;
    if slots.pending.is_empty() {
        return Err(vec![qvsmsg::UPDATE_NOTHING_PENDING.issue()]);
    }
    c.pid1()
        .cancel_slot()
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e.to_string()])])?
        .map_err(|e| vec![qvsmsg::UPDATE_RECORD.with(&[&e])])?;
    let mut kept = c.kept();
    let h = kept.of(&slots.pending);
    h.last = Attempt::Never;
    h.time = now();
    c.keep(&kept);
    c.post(
        &refcode::UPDATE_CANCELLED,
        &format!("slot {}", slots.pending),
        &[refcode::about::slot(&slots.pending)],
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slot_is_written_and_read_back_and_the_other_left_alone() {
        let names = &appliance::LoadSourceNames::DEMO;
        let (a, b) = (vec![0xAAu8; 200_000], vec![0xBBu8; 150_000]);
        let esp = loadsource::prepare::esp_image(names, &a, &b).unwrap();
        let path = std::env::temp_dir().join(format!("update-esp-{}", std::process::id()));
        fs::write(&path, &esp).unwrap();
        let new_b = vec![0xCCu8; 300_000];
        write_slot(&path, names.slot_b_path, &new_b).unwrap();
        let image = fs::read(&path).unwrap();
        assert_eq!(
            diskimg::fat::read_file(&image, names.slot_b_path).unwrap(),
            new_b
        );
        assert_eq!(
            diskimg::fat::read_file(&image, loadsource::layout::SLOT_A_PATH).unwrap(),
            a,
            "the slot that runs is as it was"
        );
        // Again, shorter: the old file goes.
        write_slot(&path, names.slot_b_path, &b).unwrap();
        let image = fs::read(&path).unwrap();
        assert_eq!(
            diskimg::fat::read_file(&image, names.slot_b_path).unwrap(),
            b
        );
        fs::remove_file(&path).unwrap();
    }
}
