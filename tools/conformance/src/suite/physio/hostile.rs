// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 12: a hostile guest writes everything at the configuration
//! space of the controller it was given (`qsvshook`'s
//! `partition::pci`); neither the host nor another partition is
//! disturbed, and the controller, reset, works for the next partition.

use std::time::{Duration, Instant};

use model::profile::{IplSource, PhysicalIo};
use partd::{Calls as _, State};

use super::devices::{activate, end, located, probed, profile, release, rig};
use crate::qemu::PASSED_NVME;
use crate::qemu::image::{blank, sparse_copy};
use crate::suite::base::{Run, posted};
use crate::suite::partition::manager::{self, Spec, partd, said};
use crate::suite::partition::{QSVS_MEMORY_MIB, host_unaffected, pid_of};

/// Suite 12.
pub fn hostile(run: &Run<'_>) -> Result<String, String> {
    let release_image = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    let marker = run.file("marker.img");
    blank(&marker, 1 << 20)?;
    {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&marker)
            .and_then(|mut f| f.write_all(qsvshook::partition::PCI_MARKER))
            .map_err(|e| e.to_string())?;
    }
    let calm_disk = run.file("calm.img");
    sparse_copy(release_image, &calm_disk)?;
    // The test image's copy is /dev/vdb, the marker /dev/vdc, the calm
    // partition's /dev/vdd.
    let mut guest = rig(run, &[marker, calm_disk])?;
    let g = &mut guest;
    let nvme = located(g, PASSED_NVME, 0)?;
    release(g, &nvme)?;
    let calm = manager::partition(g, "CALM")?;
    let mut calm_spec = Spec::new(1, QSVS_MEMORY_MIB, "/dev/vdd");
    calm_spec.side = IplSource::A;
    manager::activate(g, calm, &calm_spec).map_err(|e| manager::refused(&e))?;
    manager::until(g, calm, State::Running, run.deadline())?;
    let calm_pid = pid_of(g, calm)?;
    let hostile = manager::partition(g, "HOSTILE")?;
    let mut spec = Spec::new(1, QSVS_MEMORY_MIB, crate::suite::partition::UNIT);
    spec.side = IplSource::A;
    spec.backing.push("/dev/vdc".into());
    spec.name = "HOSTILE".into();
    let mut p = spec.profile();
    p.physical_io = vec![PhysicalIo {
        location: nvme.clone(),
        required: true,
    }];
    said(partd(g)?.put_profile(hostile, p))?;
    said(partd(g)?.activate(hostile, "HOSTILE".into(), None, None))?;
    let deadline = Instant::now() + 4 * run.timeout;
    let done = qsvshook::partition::relayed("HOSTILE", "hostile-pci: done");
    // The manager answers throughout, quickly.
    let mut slowest = Duration::ZERO;
    while !g.said(&done) {
        if Instant::now() >= deadline {
            return Err("the hostile system never said it was done".into());
        }
        let asked = Instant::now();
        partd(g)?.list().map_err(|e| e.to_string())?;
        slowest = slowest.max(asked.elapsed());
        std::thread::sleep(Duration::from_millis(500));
    }
    let what = g.line_with("HOSTILE| hostile-pci: 0000:", deadline)?;
    let total = g.line_with(&done, deadline)?;
    let h = manager::status(g, hostile)?;
    let c = manager::status(g, calm)?;
    if c.state != State::Running || pid_of(g, calm)? != calm_pid {
        return Err(format!("the calm partition is {}", c.state.word()));
    }
    host_unaffected(g)?;
    // Ended, the controller is reset and given back, and works.
    end(g, hostile)?;
    g.expect(
        &posted(
            &refcode::DEVICE_RETURNED,
            &format!("{nvme}, partition HOSTILE"),
        ),
        run.deadline(),
    )?;
    let next = manager::partition(g, "NEXT")?;
    activate(g, next, profile("IO", &[(&nvme, true)]))?;
    let works = probed(g, "NEXT", "DISK nvme", run)?;
    if !works.ends_with("read back ok") {
        return Err(format!("the controller after the hostile system: {works}"));
    }
    end(g, next)?;
    end(g, calm)?;
    Ok(format!(
        "`{}`, `{}`; HOSTILE was {} after it, CALM ran on in the same VMM, the manager \
         answered in {} ms at most, every host service ready; ended, the controller was \
         reset and given back, and NEXT's system used it: `{works}`",
        what.split_once("| ").map_or(what.as_str(), |(_, l)| l),
        total.split_once("| ").map_or(total.as_str(), |(_, l)| l),
        h.state.word(),
        slowest.as_millis()
    ))
}
