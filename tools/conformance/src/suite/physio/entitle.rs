// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 14: the entitlement of a running shared partition. A capped
//! partition raised from 0.50 to 1.00 units measures its new share with
//! the same VMM; a change outside the profile's range, of a dedicated
//! partition or beyond the pool is refused by name; the next activation
//! is the profile's again.

use std::time::Instant;

use model::units::Units;
use model::{Sharing, profile::Profile};
use partd::{Calls as _, State};
use resource::{Entitlement, Pool};

use crate::qemu::session::Session;
use crate::suite::base::{Run, channel};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::partition::resources::{
    TOLERANCE, measure, run_one, spin, spinner_launch, spinners,
};
use crate::suite::partition::{host_unaffected, pid_of};

/// The pool the shared partitions are in.
const POOL: &str = "TIGHT";

/// The profile of spinner `i`: one virtual processor, capped, `units`
/// desired between 0.20 and 1.00, in [`POOL`].
fn shared(i: usize, units: u32) -> Profile {
    let mut p = spin(i, 1).shared(units, Sharing::Capped).profile();
    p.processors.minimum_units = Some(Units(20));
    p.processors.maximum_units = Some(Units(100));
    p.processors.pool = Some(POOL.into());
    p
}

/// Change `id`'s units to `units`.
fn to(
    g: &mut Session,
    id: u32,
    units: u32,
) -> Result<Result<resource::Current, Vec<qvsmsg::Issued>>, String> {
    let change = Entitlement {
        units: Some(Units(units)),
        sharing: None,
        weight: None,
    };
    partd(g)?
        .entitlement(id, change, "suite".into())
        .map_err(|e| e.to_string())
}

/// The cores `group` used, measured.
fn cores(g: &mut Session, group: &str, per_core: f64) -> Result<f64, String> {
    Ok(measure(g, &[group.to_string()])?[0].0 / per_core)
}

/// The control group's `cpu.max`.
fn max(g: &mut Session, group: &str) -> Result<String, String> {
    Ok(channel(g)?
        .text(&format!("/sys/fs/cgroup/{group}/cpu.max"))?
        .trim()
        .to_string())
}

/// The first refusal's message, when it is `wanted`.
fn refused_with(
    answer: Result<resource::Current, Vec<qvsmsg::Issued>>,
    wanted: &qvsmsg::Message,
    what: &str,
) -> Result<String, String> {
    match answer {
        Err(errors) if errors.first().is_some_and(|e| e.is(wanted)) => Ok(errors[0].to_string()),
        other => Err(format!("{what}: {other:?}")),
    }
}

/// Suite 14.
pub fn entitlement(run: &Run<'_>) -> Result<String, String> {
    let (mut guest, ids) = spinners(run, &spinner_launch(run, 3)?, 3)?;
    let g = &mut guest;
    let deadline = Instant::now() + 3 * run.timeout;
    let host = partd(g)?.resources().map_err(|e| e.to_string())?;
    let per_core = f64::from(host.threads_per_core);
    said(partd(g)?.create_pool(Pool {
        name: POOL.into(),
        cores: vec![host.cores - 1],
        maximum_units: None,
    }))?;
    let (one, two, three) = (ids[0], ids[1], ids[2]);
    // P1 at 0.50, measured.
    let p = shared(0, 50);
    said(partd(g)?.put_profile(one, p.clone()))?;
    let got = said(partd(g)?.activate(one, p.name.clone(), None, None))?;
    manager::until(g, one, State::Running, deadline)?;
    let group = resource::cgroup::partition_group(one, &got.processors);
    let pid = pid_of(g, one)?;
    let at_half = max(g, &group)?;
    let half = cores(g, &group, per_core)?;
    if (half - 0.5).abs() > 0.5 * TOLERANCE {
        return Err(format!(
            "at 0.50 units: {half:.3} of a core (cpu.max `{at_half}`)"
        ));
    }
    // Raised to 1.00 while it runs.
    to(g, one, 100)?.map_err(|e| manager::refused(&e))?;
    let at_whole = max(g, &group)?;
    let whole = cores(g, &group, per_core)?;
    if (whole - 1.0).abs() > TOLERANCE || pid_of(g, one)? != pid {
        return Err(format!(
            "at 1.00 units: {whole:.3} of a core (cpu.max `{at_whole}`), the VMM {}",
            if pid_of(g, one)? == pid {
                "the same"
            } else {
                "another"
            }
        ));
    }
    // Outside the profile's range.
    let outside = refused_with(to(g, one, 120)?, &qvsmsg::ENTITLEMENT_OUTSIDE, "1.20 units")?;
    // Beyond the pool: P2 takes 0.30 of its 1.00 once P1 is at 0.60.
    to(g, one, 60)?.map_err(|e| manager::refused(&e))?;
    let p2 = shared(1, 30);
    said(partd(g)?.put_profile(two, p2.clone()))?;
    said(partd(g)?.activate(two, p2.name, None, None))?;
    manager::until(g, two, State::Running, deadline)?;
    let short = refused_with(
        to(g, one, 80)?,
        &qvsmsg::ENTITLEMENT_POOL_SHORT,
        "0.80 units beside P2's 0.30",
    )?;
    // A dedicated partition.
    run_one(g, three, &spin(2, 1), deadline)?;
    let dedicated = refused_with(
        to(g, three, 50)?,
        &qvsmsg::ENTITLEMENT_DEDICATED,
        "a dedicated partition",
    )?;
    // The next activation is the profile's.
    for id in [three, two, one] {
        said(partd(g)?.shut_down(id))?;
        manager::ended(g, id, deadline)?;
    }
    let again = said(partd(g)?.activate(one, p.name.clone(), None, None))?;
    manager::until(g, one, State::Running, deadline)?;
    let group = resource::cgroup::partition_group(one, &again.processors);
    let next = max(g, &group)?;
    if next != at_half {
        return Err(format!(
            "the next activation's cpu.max `{next}`, the profile's `{at_half}`"
        ));
    }
    host_unaffected(g)?;
    Ok(format!(
        "P1 capped at 0.50 units used {half:.3} of a core (cpu.max `{at_half}`); raised to \
         1.00 with VMM {pid} running on, {whole:.3} (`{at_whole}`); 1.20: `{outside}`; 0.80 \
         beside P2's 0.30 in a pool of one core: `{short}`; dedicated P3: `{dedicated}`; \
         activated again, cpu.max `{next}`, the profile's"
    ))
}
