// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 13: a host's configuration captured over QSI and put back on
//! the host installed again (`docs/PHASE9.md`, Section 5). Nothing new
//! is built for it: the proof is that Phase 8's families are complete.

use std::fmt::Write as _;

use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::host::model::{Definition, EthernetAdapter, Lun, Profile, ScsiAdapter};
use qsi::host::{host as h, partition, partitions, profiles, vres};

use super::{Console, Opts, console, host};
use crate::suite::base::Run;
use crate::suite::install::{PASSWORD, USER, sign_on};
use crate::suite::qsi::families::profile;
use poold::Calls as _;

use crate::suite::storage::{disk_names, pool_until, poold, said};

/// The pool and the switch, by the names the document keeps.
const POOL: &str = "POOL1";
const SWITCH: &str = "LAN";

/// A host's configuration as QSI reads it: one document.
#[derive(Clone, Debug, PartialEq)]
struct Captured {
    /// The pools and switches it depends on, by name.
    pools: Vec<String>,
    switches: Vec<String>,
    /// Each shared processor pool.
    proc_pools: Vec<h::ProcPool>,
    /// Each virtual disk: its pool, name, size and kind.
    vdisks: Vec<(String, String, u64, bool)>,
    /// Each tape volume: its name, identifier and capacity.
    volumes: Vec<(String, String, u64)>,
    /// Each partition's definition and its profiles.
    partitions: Vec<(Definition, Vec<Profile>)>,
}

impl Captured {
    /// As one document, to be kept.
    fn document(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "pools {:?}\nswitches {:?}", self.pools, self.switches);
        for p in &self.proc_pools {
            let _ = writeln!(out, "processor pool {p:?}");
        }
        for v in &self.vdisks {
            let _ = writeln!(out, "virtual disk {v:?}");
        }
        for v in &self.volumes {
            let _ = writeln!(out, "tape volume {v:?}");
        }
        for (d, ps) in &self.partitions {
            let _ = writeln!(out, "partition {d:?}");
            for p in ps {
                let _ = writeln!(out, "  profile {p:?}");
            }
        }
        out
    }
}

fn read(c: &mut Console) -> Result<Captured, String> {
    let ask = |c: &mut Console, r| c.ask(r);
    let AnyReply::Vres(vres::Reply::Pools(pools)) =
        ask(c, AnyRequest::Vres(vres::Request::Pools(Empty)))?
    else {
        return Err("pools".into());
    };
    let AnyReply::Vres(vres::Reply::Switches(switches)) =
        ask(c, AnyRequest::Vres(vres::Request::Switches(Empty)))?
    else {
        return Err("switches".into());
    };
    let AnyReply::Host(h::Reply::Host(resources)) =
        ask(c, AnyRequest::Host(h::Request::Host(Empty)))?
    else {
        return Err("resources".into());
    };
    let AnyReply::Vres(vres::Reply::Vdisks(vdisks)) = ask(
        c,
        AnyRequest::Vres(vres::Request::Vdisks(vres::Vdisks {
            pool: String::new(),
        })),
    )?
    else {
        return Err("vdisks".into());
    };
    let AnyReply::Vres(vres::Reply::Volumes(volumes)) =
        ask(c, AnyRequest::Vres(vres::Request::Volumes(Empty)))?
    else {
        return Err("volumes".into());
    };
    let AnyReply::Partitions(partitions::Reply::List(all)) =
        ask(c, AnyRequest::Partitions(partitions::Request::List(Empty)))?
    else {
        return Err("partitions".into());
    };
    let mut parts = Vec::new();
    for s in all {
        let AnyReply::Partitions(partitions::Reply::Get(got)) = ask(
            c,
            AnyRequest::Partitions(partitions::Request::Get(partitions::Get { id: s.id })),
        )?
        else {
            return Err("get".into());
        };
        let AnyReply::Profiles(profiles::Reply::List(names)) = ask(
            c,
            AnyRequest::Profiles(profiles::Request::List(profiles::List { id: s.id })),
        )?
        else {
            return Err("profiles".into());
        };
        let mut ps = Vec::new();
        for n in names {
            let AnyReply::Profiles(profiles::Reply::Get(p)) = ask(
                c,
                AnyRequest::Profiles(profiles::Request::Get(profiles::Get {
                    id: s.id,
                    name: n.name.clone(),
                })),
            )?
            else {
                return Err("profile".into());
            };
            ps.push(p.profile);
        }
        parts.push((got.definition, ps));
    }
    Ok(Captured {
        pools: pools.into_iter().map(|p| p.name).collect(),
        switches: switches.into_iter().map(|s| s.name).collect(),
        proc_pools: resources.pools,
        vdisks: vdisks
            .into_iter()
            .map(|d| (d.pool, d.name, d.bytes, d.thick))
            .collect(),
        volumes: volumes
            .into_iter()
            .map(|v| (v.name, v.identifier, v.capacity_mib))
            .collect(),
        partitions: parts,
    })
}

/// The pool and the switch made at the host, by the same names: the
/// pool through the pool service, the switch at Service Tools.
fn base(rig: &mut super::Rig<'_>, run: &Run<'_>) -> Result<(), String> {
    let names = disk_names(&mut rig.q.guest, 1)?;
    said(poold(&mut rig.q.guest)?.create_pool(
        POOL.into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(&mut rig.q.guest, POOL, run.deadline(), |p| {
        !p.mount.is_empty()
    })?;
    said(poold(&mut rig.q.guest)?.designate(POOL.into()))?;
    let deadline = run.deadline();
    sign_on(&mut rig.q.guest, USER, PASSWORD, deadline)?;
    crate::suite::network::tools::to_switches(&mut rig.q.guest, deadline)?;
    crate::suite::network::tools::make(&mut rig.q.guest, SWITCH, "1 10", "1500", deadline)?;
    let _ = rig.q.guest.type_keys(crate::qemu::term::key::F3);
    Ok(())
}

/// A profile of `name` on `disk` and the switch: the image's QS/400
/// profile with its storage and Ethernet the host's.
fn of(name: &str, disk: &str, shared: bool) -> Profile {
    let mut p = profile(name);
    p.scsi = vec![ScsiAdapter {
        slot: 2,
        luns: vec![Lun {
            lun: 0,
            backing: format!("{POOL}/{disk}"),
            serial: None,
        }],
    }];
    p.ethernet = vec![EthernetAdapter {
        slot: 4,
        switch: Some(SWITCH.into()),
        vlan: 10,
        tagged: Vec::new(),
        mac: None,
        required: false,
    }];
    if shared {
        p.processors.mode = qsi::host::model::ProcessorMode::Shared;
        p.processors.minimum_units = Some(10);
        p.processors.desired_units = Some(50);
        p.processors.maximum_units = Some(100);
        p.processors.sharing = Some(qsi::host::model::Sharing::Uncapped);
        p.processors.weight = Some(128);
        p.processors.pool = Some("SHARED1".into());
    }
    p
}

/// Put the document back over QSI on a host that has its pool and
/// switch: the processor pools, the disks and volumes, then each
/// partition made, its definition changed and its profiles put.
fn put_back(c: &mut Console, doc: &Captured) -> Result<(), String> {
    for p in &doc.proc_pools {
        c.ask(AnyRequest::Vres(vres::Request::ProcPoolCreate(
            vres::ProcPoolSpec {
                name: p.name.clone(),
                cores: p.cores.clone(),
                maximum_units: Some(p.maximum),
            },
        )))?;
    }
    for (pool, name, bytes, thick) in &doc.vdisks {
        c.ask(AnyRequest::Vres(vres::Request::VdiskCreate(
            vres::VdiskCreate {
                pool: pool.clone(),
                name: name.clone(),
                mib: bytes / (1 << 20),
                thick: *thick,
            },
        )))?;
    }
    for (name, identifier, mib) in &doc.volumes {
        c.ask(AnyRequest::Vres(vres::Request::VolumeCreate(
            vres::VolumeCreate {
                name: name.clone(),
                identifier: identifier.clone(),
                mib: *mib,
            },
        )))?;
    }
    for (d, ps) in &doc.partitions {
        let AnyReply::Partition(partition::Reply::Create(made)) = c.ask(AnyRequest::Partition(
            partition::Request::Create(partition::Create {
                name: d.name.clone(),
            }),
        ))?
        else {
            return Err("create".into());
        };
        if made.id != d.id {
            return Err(format!(
                "{} made as {}, captured as {}",
                d.name, made.id, d.id
            ));
        }
        for p in ps {
            c.ask(AnyRequest::Profiles(profiles::Request::Put(
                profiles::Put {
                    id: made.id,
                    profile: p.clone(),
                    revision: None,
                },
            )))?;
        }
        let mut back = d.clone();
        back.created = made.created;
        c.ask(AnyRequest::Partition(partition::Request::Change(
            partition::Change { definition: back },
        )))?;
    }
    Ok(())
}

/// Suite 13: three partitions, captured, the host installed again, put
/// back, and every definition and profile read back the same.
#[allow(clippy::too_many_lines)] // one scenario, in order
pub fn capture(run: &Run<'_>) -> Result<String, String> {
    let opts = Opts {
        pool_disks: 1,
        ..Opts::default()
    };
    let mut rig = host(run, &opts)?;
    base(&mut rig, run)?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    c.ask(AnyRequest::Vres(vres::Request::ProcPoolCreate(
        vres::ProcPoolSpec {
            name: "SHARED1".into(),
            cores: vec![1],
            maximum_units: Some(100),
        },
    )))?;
    for (name, mib, thick) in [("VD1", 64, false), ("VD2", 32, true), ("VD3", 16, false)] {
        c.ask(AnyRequest::Vres(vres::Request::VdiskCreate(
            vres::VdiskCreate {
                pool: POOL.into(),
                name: name.into(),
                mib,
                thick,
            },
        )))?;
    }
    c.ask(AnyRequest::Vres(vres::Request::VolumeCreate(
        vres::VolumeCreate {
            name: "TAPE01".into(),
            identifier: "T00001".into(),
            mib: 64,
        },
    )))?;
    for (i, name) in ["PROD", "TEST", "DEV"].iter().enumerate() {
        let id = crate::suite::qsi::flows::make_partition(&mut c.link, name)?;
        let disk = format!("VD{}", i + 1);
        c.ask(AnyRequest::Profiles(profiles::Request::Put(
            profiles::Put {
                id,
                profile: of("NORMAL", &disk, i == 1),
                revision: None,
            },
        )))?;
    }
    let captured = read(&mut c)?;
    let document = captured.document();
    let kept = run.file("captured.txt");
    std::fs::write(&kept, &document).map_err(|e| e.to_string())?;
    c.link.close();
    crate::suite::install::power_off(&mut rig.q.guest)?;
    drop(rig);
    // Installed again: a new load source from the ISO, a blank pool
    // disk; the pool and the switch made again by the same names.
    let mut rig = host(run, &opts)?;
    base(&mut rig, run)?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    put_back(&mut c, &captured)?;
    let again = read(&mut c)?;
    c.link.close();
    let mut gaps = Vec::new();
    for ((d0, p0), (d1, p1)) in captured.partitions.iter().zip(&again.partitions) {
        let mut d1c = d1.clone();
        if d1c.created != d0.created {
            gaps.push(format!(
                "{}: created {} not {}",
                d0.name, d1c.created, d0.created
            ));
            d1c.created = d0.created;
        }
        if &d1c != d0 {
            return Err(format!("definition {}:\n{d0:?}\n{d1:?}", d0.name));
        }
        if p0 != p1 {
            return Err(format!("profiles of {}:\n{p0:?}\n{p1:?}", d0.name));
        }
    }
    if captured.partitions.len() != again.partitions.len()
        || captured.vdisks != again.vdisks
        || captured.volumes != again.volumes
        || captured
            .proc_pools
            .iter()
            .map(|p| (&p.name, &p.cores))
            .collect::<Vec<_>>()
            != again
                .proc_pools
                .iter()
                .map(|p| (&p.name, &p.cores))
                .collect::<Vec<_>>()
    {
        return Err(format!(
            "captured:\n{document}\nread back:\n{}",
            again.document()
        ));
    }
    let macs: Vec<String> = again
        .partitions
        .iter()
        .flat_map(|(_, ps)| {
            ps.iter()
                .flat_map(|p| p.ethernet.iter().filter_map(|e| e.mac.clone()))
        })
        .collect();
    let serials: Vec<String> = again
        .partitions
        .iter()
        .map(|(d, _)| d.serial.clone())
        .collect();
    Ok(format!(
        "captured over QSI ({} bytes, {}): 3 partitions, {} profiles, processor pool SHARED1, \
         {} virtual disks, {} tape volume, pool {POOL} and switch {SWITCH}; the host installed \
         again, its pool and switch made again, the document put back over QSI; every definition \
         and profile read back the same, serials {serials:?} and MACs {macs:?} included; {}",
        document.len(),
        kept.display(),
        captured
            .partitions
            .iter()
            .map(|(_, p)| p.len())
            .sum::<usize>(),
        captured.vdisks.len(),
        captured.volumes.len(),
        if gaps.is_empty() {
            "no gap".to_string()
        } else {
            format!("what QSI cannot put back: {}", gaps.join("; "))
        }
    ))
}
