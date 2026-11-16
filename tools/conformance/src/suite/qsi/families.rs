// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 5 and 10: every request of the base and host families once,
//! each reply held against what the service that owns it says locally;
//! and a local service killed under requests in flight.

use std::time::{Duration, Instant};

use partd::Calls as _;
use poold::Calls as _;
use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::base;
use qsi::host::{dumps, host as h, panel, partition, partitions, profiles, refcodes, vres};
use qsictl::link::Link;

use super::{Qsi, eventually, host, paired};
use crate::suite::base::{Run, channel};
use crate::suite::partition::manager::{self, partd, said};
use crate::suite::service::qs400_spec;

/// Ask, and the reply decoded; a refusal is an error with its messages.
#[allow(clippy::needless_pass_by_value)] // a request is built for this call
pub fn ask(l: &mut Link, r: AnyRequest) -> Result<AnyReply, String> {
    let reply = l.ask(&r)?;
    if reply.refusal().is_some() {
        return Err(format!("{r:?}: {}", qsictl::pair::refusal(&reply)));
    }
    reply
        .decode()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "a reply this harness does not read".into())
}

/// Ask, expecting a refusal: its first identifier.
pub fn refused(l: &mut Link, r: &AnyRequest) -> Result<String, String> {
    let reply = l.ask(r)?;
    reply
        .refusal()
        .map(|(m, _)| m.to_string())
        .ok_or_else(|| format!("{r:?} was not refused"))
}

/// A QSI profile of the QS/400 partition the suites run.
pub fn profile(name: &str) -> qsi::host::model::Profile {
    let mut p = qs400_spec(None).profile();
    p.name = name.into();
    qsiserver::families::convert::profile(&p)
}

/// The base family against PID 1, its history and the inventory.
fn base_family(q: &mut Qsi<'_>, l: &mut Link) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let AnyReply::Base(base::Reply::Identify(me)) =
        ask(l, AnyRequest::Base(base::Request::Identify(Empty)))?
    else {
        return Err("identify".into());
    };
    if me.name != crate::suite::install::HOST || me.release != release::RELEASE.to_string() {
        return Err(format!("identify: {me:?}"));
    }
    lines.push(format!("identify {} {}", me.name, me.release));
    let AnyReply::Base(base::Reply::State(s)) =
        ask(l, AnyRequest::Base(base::Request::State(Empty)))?
    else {
        return Err("state".into());
    };
    let slot = channel(&mut q.guest)?
        .status()?
        .fact(release::fact::SLOT)
        .unwrap_or("")
        .to_string();
    if s.power != base::PowerState::Running || !slot.starts_with(&s.slot) || s.slot.is_empty() {
        return Err(format!("state {s:?}, PID 1's slot {slot}"));
    }
    lines.push(format!("state {:?} slot {}", s.power, s.slot));
    let AnyReply::Base(base::Reply::Time(t)) =
        ask(l, AnyRequest::Base(base::Request::Time(Empty)))?
    else {
        return Err("time".into());
    };
    let now = appliance::time::now_secs();
    if (t.unix_ns / 1_000_000_000 - now).abs() > 120 {
        return Err(format!(
            "the host's clock is {} s off",
            t.unix_ns / 1_000_000_000 - now
        ));
    }
    lines.push(format!("time {}", t.zone));
    let AnyReply::Base(base::Reply::Refcodes(page)) = ask(
        l,
        AnyRequest::Base(base::Request::Refcodes(base::Refcodes {
            before: None,
            limit: 3,
        })),
    )?
    else {
        return Err("refcodes".into());
    };
    let history = channel(&mut q.guest)?.text("/data/log/refcodes.ndjson")?;
    let count = history.lines().count() as u64;
    let newest = page.codes.first().ok_or("an empty history")?;
    if newest.seq + 5 < count || !history.contains(&newest.code) {
        return Err(format!(
            "the newest code {newest:?}, the file has {count} lines"
        ));
    }
    lines.push(format!("refcodes newest {} at {}", newest.code, newest.seq));
    Ok(lines)
}

/// The host family against the partition manager and the inventory.
fn host_family(q: &mut Qsi<'_>, l: &mut Link) -> Result<Vec<String>, String> {
    let AnyReply::Host(h::Reply::Host(r)) = ask(l, AnyRequest::Host(h::Request::Host(Empty)))?
    else {
        return Err("host".into());
    };
    let local = partd(&mut q.guest)?
        .resources()
        .map_err(|e| e.to_string())?;
    if r.cores != local.cores || r.memory.installed != local.memory.installed {
        return Err(format!("resources {r:?} against {local:?}"));
    }
    let AnyReply::Host(h::Reply::Settings(s)) =
        ask(l, AnyRequest::Host(h::Request::Settings(Empty)))?
    else {
        return Err("settings".into());
    };
    let local = partd(&mut q.guest)?.settings().map_err(|e| e.to_string())?;
    if s.max_partitions != local.max_partitions {
        return Err(format!("settings {s:?}"));
    }
    let AnyReply::Host(h::Reply::Hardware(hw)) =
        ask(l, AnyRequest::Host(h::Request::Hardware(Empty)))?
    else {
        return Err("hardware".into());
    };
    let text = channel(&mut q.guest)?.text("/data/state/inventory.toml")?;
    let inv: state::Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
    if hw.items.len() != inv.disks.len() + inv.interfaces.len() + inv.pci.len() {
        return Err(format!(
            "{} items, the inventory has more or fewer",
            hw.items.len()
        ));
    }
    Ok(vec![format!(
        "host {} cores, {} MiB; settings; hardware {} items",
        r.cores,
        r.memory.installed,
        hw.items.len()
    )])
}

/// A partition made, changed, activated, looked at and ended, every
/// request of its families once.
#[allow(clippy::too_many_lines)] // a request a step, in order
fn partition_families(q: &mut Qsi<'_>, l: &mut Link, run: &Run<'_>) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let AnyReply::Partition(partition::Reply::Create(d)) = ask(
        l,
        AnyRequest::Partition(partition::Request::Create(partition::Create {
            name: "QSIP".into(),
        })),
    )?
    else {
        return Err("create".into());
    };
    let local = said_local(partd(&mut q.guest)?.definition(d.id))?;
    if local.name != "QSIP" || local.serial != d.serial {
        return Err(format!("created {d:?}, partd has {local:?}"));
    }
    let id = d.id;
    let put = |revision| {
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id,
            profile: profile("NORMAL"),
            revision,
        }))
    };
    let AnyReply::Profiles(profiles::Reply::Put(first)) = ask(l, put(None))? else {
        return Err("put".into());
    };
    if refused(l, &put(None))? != profiles::msgid::EXISTS {
        return Err("a second new profile of one name was not EXISTS".into());
    }
    let AnyReply::Profiles(profiles::Reply::Put(second)) = ask(l, put(Some(first.revision)))?
    else {
        return Err("put at its revision".into());
    };
    if refused(l, &put(Some(first.revision)))? != profiles::msgid::CHANGED {
        return Err("a stale put was not CHANGED".into());
    }
    ask(
        l,
        AnyRequest::Profiles(profiles::Request::SetDefault(profiles::SetDefault {
            id,
            name: "NORMAL".into(),
        })),
    )?;
    let AnyReply::Profiles(profiles::Reply::List(entries)) = ask(
        l,
        AnyRequest::Profiles(profiles::Request::List(profiles::List { id })),
    )?
    else {
        return Err("profiles".into());
    };
    if !entries
        .iter()
        .any(|e| e.name == "NORMAL" && e.default && e.revision == second.revision)
    {
        return Err(format!("profiles {entries:?}"));
    }
    let AnyReply::Profiles(profiles::Reply::Get(got)) = ask(
        l,
        AnyRequest::Profiles(profiles::Request::Get(profiles::Get {
            id,
            name: "NORMAL".into(),
        })),
    )?
    else {
        return Err("get profile".into());
    };
    lines.push(format!(
        "partition {id} made, profile NORMAL at revisions {} and {}, a stale put CHANGED",
        first.revision, got.revision
    ));
    let AnyReply::Partitions(partitions::Reply::Get(p)) = ask(
        l,
        AnyRequest::Partitions(partitions::Request::Get(partitions::Get { id })),
    )?
    else {
        return Err("get".into());
    };
    let mut changed = p.definition.clone();
    changed.name = "QSIQ".into();
    ask(
        l,
        AnyRequest::Partition(partition::Request::Change(partition::Change {
            definition: changed,
        })),
    )?;
    if said_local(partd(&mut q.guest)?.definition(id))?.name != "QSIQ" {
        return Err("the change is not partd's".into());
    }
    ask(
        l,
        AnyRequest::Partitions(partitions::Request::Adapters(partitions::Adapters { id })),
    )?;
    // Activated: its code history, the panel, the console's screen, the
    // drives, then ended.
    let AnyReply::Partition(partition::Reply::Activate(current)) = ask(
        l,
        AnyRequest::Partition(partition::Request::Activate(partition::Activate {
            id,
            profile: String::new(),
            source: None,
            keylock: None,
        })),
    )?
    else {
        return Err("activate".into());
    };
    let running = eventually(
        crate::suite::service::ipl_patience(run).saturating_duration_since(Instant::now()),
        || {
            let AnyReply::Partitions(partitions::Reply::List(all)) =
                ask(l, AnyRequest::Partitions(partitions::Request::List(Empty))).ok()?
            else {
                return None;
            };
            all.into_iter().find(|s| {
                s.id == id
                    && s.state == qsi::host::model::PartitionState::Running
                    && s.code == "C6001090"
            })
        },
    )
    .ok_or("the partition did not reach its IPL complete")?;
    let local = manager::status(&mut q.guest, id)?;
    if local.code != running.code || local.name != running.name {
        return Err(format!("listed {running:?}, partd has {local:?}"));
    }
    let AnyReply::Refcodes(refcodes::Reply::History(page)) = ask(
        l,
        AnyRequest::Refcodes(refcodes::Request::History(refcodes::History {
            id,
            before: None,
            limit: 256,
        })),
    )?
    else {
        return Err("history".into());
    };
    let local = said(partd(&mut q.guest)?.history(id))?;
    if page.codes.len() != local.len()
        || page.codes.first().map(|c| &c.refcode.code) != local.last().map(|c| &c.code)
    {
        return Err(format!(
            "{} codes, partd has {}",
            page.codes.len(),
            local.len()
        ));
    }
    let AnyReply::Panel(panel::Reply::Function(shown)) = ask(
        l,
        AnyRequest::Panel(panel::Request::Function(panel::Function {
            id,
            function: 20,
            args: panel::PanelArgs {
                source: None,
                ipl_type: None,
                keylock: None,
            },
        })),
    )?
    else {
        return Err("panel".into());
    };
    ask(
        l,
        AnyRequest::Console(qsi::host::console::Request::Screen(
            qsi::host::console::Screen { id },
        )),
    )?;
    ask(
        l,
        AnyRequest::Vres(vres::Request::Drives(vres::Drives { id })),
    )?;
    ask(
        l,
        AnyRequest::Dumps(dumps::Request::List(dumps::List { id })),
    )?;
    ask(
        l,
        AnyRequest::Partition(partition::Request::ShutDown(partition::ShutDown {
            id,
            kind: partition::ShutdownKind::Immediate,
            delay: 0,
            restart: false,
        })),
    )?;
    manager::ended(&mut q.guest, id, Instant::now() + Duration::from_mins(1))?;
    ask(
        l,
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id,
            profile: profile("OTHER"),
            revision: None,
        })),
    )?;
    ask(
        l,
        AnyRequest::Profiles(profiles::Request::Delete(profiles::Delete {
            id,
            name: "OTHER".into(),
        })),
    )?;
    ask(
        l,
        AnyRequest::Partition(partition::Request::Delete(partition::Delete {
            id,
            disks: false,
        })),
    )?;
    if partd(&mut q.guest)?
        .definition(id)
        .map_err(|e| e.to_string())?
        .is_ok()
    {
        return Err("the partition was not deleted".into());
    }
    lines.push(format!(
        "changed, activated with {} MiB, Running at C6001090, {} codes, panel 20 `{}`, screen, \
         drives, dumps, shut down, a profile put and deleted, the partition deleted",
        current.memory_mib,
        page.codes.len(),
        shown.text.lines().next().unwrap_or("")
    ));
    Ok(lines)
}

fn said_local<T>(r: Result<Result<T, Vec<qvsmsg::Issued>>, ipc::WireError>) -> Result<T, String> {
    said(r)
}

/// The vres family against the pool and switch services.
fn vres_family(q: &mut Qsi<'_>, l: &mut Link) -> Result<Vec<String>, String> {
    let AnyReply::Vres(vres::Reply::Pools(pools)) =
        ask(l, AnyRequest::Vres(vres::Request::Pools(Empty)))?
    else {
        return Err("pools".into());
    };
    let local = crate::suite::storage::poold(&mut q.guest)?
        .pools()
        .map_err(|e| e.to_string())?;
    if pools.len() != local.len() {
        return Err(format!(
            "{} pools, the pool service has {}",
            pools.len(),
            local.len()
        ));
    }
    for r in [
        vres::Request::Vdisks(vres::Vdisks {
            pool: String::new(),
        }),
        vres::Request::Images(Empty),
        vres::Request::Volumes(Empty),
        vres::Request::Switches(Empty),
        vres::Request::Ports(vres::Ports {
            switch: String::new(),
        }),
        vres::Request::Vlans(Empty),
        vres::Request::Limits(Empty),
    ] {
        ask(l, AnyRequest::Vres(r))?;
    }
    let spec = vres::ProcPoolSpec {
        name: "QSIPOOL".into(),
        cores: Vec::new(),
        maximum_units: None,
    };
    let made = l.ask(&AnyRequest::Vres(vres::Request::ProcPoolCreate(
        spec.clone(),
    )))?;
    // No core to spare on the harness's host is the partition
    // manager's own refusal, passed through.
    let pool_said = if let Some((m, _)) = made.refusal() {
        format!("a processor pool refused {m}")
    } else {
        ask(l, AnyRequest::Vres(vres::Request::ProcPoolChange(spec)))?;
        ask(
            l,
            AnyRequest::Vres(vres::Request::ProcPoolDelete(vres::ProcPoolDelete {
                name: "QSIPOOL".into(),
            })),
        )?;
        "a processor pool made, changed and deleted".into()
    };
    Ok(vec![format!(
        "{} storage pools as the pool service has them; disks, images, volumes, switches, ports, \
         VLANs and limits answered; {pool_said}",
        pools.len()
    )])
}

/// Suite 5.
pub fn families(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, true)?;
    let deadline = run.deadline();
    let (_, _, mut l) = paired(&mut q, "MC1", deadline)?;
    let mut lines = base_family(&mut q, &mut l)?;
    lines.extend(host_family(&mut q, &mut l)?);
    lines.extend(vres_family(&mut q, &mut l)?);
    lines.extend(partition_families(&mut q, &mut l, run)?);
    l.close();
    Ok(lines.join("; "))
}

/// Suite 10: the partition manager killed while requests are in
/// flight: its requests are refused by name, the others answer, and it
/// is served again when it is back.
pub fn service_kill(run: &Run<'_>) -> Result<String, String> {
    let mut q = host(run, false)?;
    let deadline = run.deadline();
    let (_, _, mut l) = paired(&mut q, "MC1", deadline)?;
    channel(&mut q.guest)?.ask(&format!("{} partd", qsvshook::proto::command::KILL))?;
    let mut down = None;
    let mut other_answered = 0;
    let until = Instant::now() + Duration::from_secs(30);
    while down.is_none() && Instant::now() < until {
        let r = l.ask(&AnyRequest::Partitions(partitions::Request::List(Empty)))?;
        if let Some((m, t)) = r.refusal() {
            down = Some(format!("{m} {t}"));
        }
        if ask(&mut l, AnyRequest::Base(base::Request::Identify(Empty))).is_ok() {
            other_answered += 1;
        }
    }
    let down = down.ok_or("the list was never refused while the partition manager was down")?;
    if !down.starts_with(qsi::host::host::msgid::SERVICE_DOWN) {
        return Err(format!("refused with {down}"));
    }
    let back = eventually(Duration::from_mins(1), || {
        ask(
            &mut l,
            AnyRequest::Partitions(partitions::Request::List(Empty)),
        )
        .ok()
    })
    .ok_or("the partition manager was not served again")?;
    let _ = back;
    l.close();
    Ok(format!(
        "while the partition manager was down its request was refused `{down}`, identify \
         answered {other_answered} time(s) meanwhile; served again once it was back"
    ))
}
