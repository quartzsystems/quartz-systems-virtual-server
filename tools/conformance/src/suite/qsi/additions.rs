// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsi-additions`: QS/400's handoff 24 from a console (QSI's bits 19
//! to 26), each request held against what the service that owns it says
//! locally and each refusal against its identifiers, QSI's then the
//! host's; then the host restarted, and the pool and the switch made
//! over QSI still there, over QSI and at Service Tools.

use std::time::{Duration, Instant};

use partd::Calls as _;
use poold::Calls as _;
use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::host::{dumps, host as h, panel, partition, partitions, profiles, vres};
use qsi::{base, service};
use qsictl::link::Link;

use crate::qemu::term::key;
use vswitchd::Calls as _;

use super::families::{ask, profile};
use super::{Qsi, eventually, host_disks, link, paired};
use crate::suite::base::{Run, channel};
use crate::suite::install::{PASSWORD, USER, option, power_off, sign_on, titled};
use crate::suite::network::vswitchd;
use crate::suite::partition::manager::partd;
use crate::suite::storage::poold;

/// Ask, expecting a refusal: QSI's identifier first, then the host's,
/// each named in the refusal.
fn refused_as(l: &mut Link, r: &AnyRequest, qsi: &str, host: &str) -> Result<String, String> {
    let reply = l.ask(r)?;
    let Some((first, _)) = reply.refusal() else {
        return Err(format!("{r:?} was not refused"));
    };
    let all = qsictl::pair::refusal(&reply);
    if first != qsi || !all.contains(host) {
        return Err(format!("{r:?}: refused `{all}`, not {qsi} then {host}"));
    }
    Ok(format!("{qsi} then {host}"))
}

/// Bits 20 and 19: a pool made, renamed and designated, one extended,
/// one of a bad name; a switch made, changed, renamed, the management
/// interface refused as its uplink, one of a bad name.
#[allow(clippy::too_many_lines)] // one family's steps, told in order
fn pools_and_switches(q: &mut Qsi<'_>, l: &mut Link) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let AnyReply::Vres(vres::Reply::Disks(disks)) =
        ask(l, AnyRequest::Vres(vres::Request::Disks(Empty)))?
    else {
        return Err("disks".into());
    };
    let local = poold(&mut q.guest)?
        .candidates()
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = disks.iter().map(|d| d.name.clone()).collect();
    let locals: Vec<String> = local.iter().map(|d| d.name.clone()).collect();
    if names != locals || names.len() < 2 {
        return Err(format!(
            "the disks a pool may take: {names:?}, locally {locals:?}"
        ));
    }
    lines.push(format!(
        "{} disks a pool may take, as the pool service says",
        names.len()
    ));
    lines.push(refused_as(
        l,
        &AnyRequest::Vres(vres::Request::PoolCreate(vres::PoolCreate {
            name: "9BAD".into(),
            protection: vres::Protection::Mirrored,
            disks: names[..2].to_vec(),
            spares: Vec::new(),
            confirm: Vec::new(),
        })),
        vres::pools::BAD_NAME,
        "QVS3001",
    )?);
    let AnyReply::Vres(vres::Reply::PoolCreate(made)) = ask(
        l,
        AnyRequest::Vres(vres::Request::PoolCreate(vres::PoolCreate {
            name: "QSIP1".into(),
            protection: vres::Protection::Mirrored,
            disks: names[..2].to_vec(),
            spares: Vec::new(),
            confirm: Vec::new(),
        })),
    )?
    else {
        return Err("pool create".into());
    };
    ask(
        l,
        AnyRequest::Vres(vres::Request::PoolRename(vres::PoolRename {
            name: "QSIP1".into(),
            to: "QSIPOOL".into(),
        })),
    )?;
    ask(
        l,
        AnyRequest::Vres(vres::Request::Designate(vres::Designate {
            pool: "QSIPOOL".into(),
        })),
    )?;
    let local = poold(&mut q.guest)?.pools().map_err(|e| e.to_string())?;
    if !local.iter().any(|p| p.name == "QSIPOOL" && p.repository) {
        return Err(format!("the pool service has {local:?}"));
    }
    lines.push(format!(
        "pool {} made {}, renamed QSIPOOL and designated, as the pool service has it",
        made.name, made.protection
    ));
    lines.push(refused_as(
        l,
        &AnyRequest::Vres(vres::Request::PoolExtend(vres::PoolExtend {
            pool: "QSIPOOL".into(),
            disks: Vec::new(),
            confirm: Vec::new(),
        })),
        vres::pools::CANNOT_GROW,
        "QVS3019",
    )?);

    let spec = |name: &str, vlans: Vec<u16>| vres::SwitchSpec {
        name: name.into(),
        vlans,
        mtu: 0,
    };
    lines.push(refused_as(
        l,
        &AnyRequest::Vres(vres::Request::SwitchCreate(spec("bad name", vec![1]))),
        vres::switches::BAD_NAME,
        "QVS4001",
    )?);
    ask(
        l,
        AnyRequest::Vres(vres::Request::SwitchCreate(spec("QSIS1", vec![1]))),
    )?;
    ask(
        l,
        AnyRequest::Vres(vres::Request::SwitchChange(spec("QSIS1", vec![1, 20]))),
    )?;
    ask(
        l,
        AnyRequest::Vres(vres::Request::SwitchRename(vres::SwitchRename {
            name: "QSIS1".into(),
            to: "QSISW".into(),
        })),
    )?;
    let management = vswitchd(&mut q.guest)?
        .interfaces()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|i| i.management)
        .ok_or("no management interface")?;
    lines.push(refused_as(
        l,
        &AnyRequest::Vres(vres::Request::UplinkSet(vres::UplinkSet {
            switch: "QSISW".into(),
            uplink: vres::Uplink {
                interfaces: vec![management.location.clone()],
                bond: None,
                native_vlan: None,
            },
        })),
        vres::switches::MANAGEMENT,
        "QVS4102",
    )?);
    let local = vswitchd(&mut q.guest)?
        .switches()
        .map_err(|e| e.to_string())?;
    let sw = local
        .iter()
        .filter_map(|s| s.switch.as_ref())
        .find(|s| s.name == "QSISW")
        .ok_or_else(|| format!("the switch service has {local:?}"))?;
    if sw.vlans != [1, 20] || sw.uplink.is_some() {
        return Err(format!("switch QSISW is {sw:?}"));
    }
    lines.push("switch QSISW made, its VLANs changed, renamed, with no uplink".into());
    Ok(lines)
}

/// Bits 21, 22 and 24: a partition made at an ID with its settings, its
/// settings changed, an ID taken and one past the limit refused; its
/// panel kept from a console; the functions the host has; the
/// activations and a profile's last use.
#[allow(clippy::too_many_lines)] // one family's steps, told in order
fn partitions_v3(q: &mut Qsi<'_>, l: &mut Link) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    let settings = |panel: bool, delay: u32| partition::Settings {
        description: "made over QSI".into(),
        panel,
        with_host: partition::WithHost::Delayed,
        delay,
    };
    let create = |id: u32, name: &str| {
        AnyRequest::Partition(partition::Request::CreateWith(partition::CreateWith {
            id: Some(id),
            name: name.into(),
            settings: settings(false, 30),
        }))
    };
    let AnyReply::Partition(partition::Reply::CreateWith(d)) = ask(l, create(7, "QSIP7"))? else {
        return Err("create with".into());
    };
    if d.id != 7 {
        return Err(format!("made at {}", d.id));
    }
    let local = partd(&mut q.guest)?
        .partition_settings(7)
        .map_err(|e| e.to_string())?
        .map_err(|e| crate::suite::partition::manager::refused(&e))?;
    if local.panel || local.delay_secs != 30 || local.description != "made over QSI" {
        return Err(format!("the partition manager keeps {local:?}"));
    }
    lines.push(
        "partition 7 made at its ID with its settings, as the partition manager keeps them".into(),
    );
    lines.push(refused_as(
        l,
        &create(7, "QSIP8"),
        partition::msgid::ID_TAKEN,
        "QVS2016",
    )?);
    lines.push(refused_as(
        l,
        &create(998, "QSIP9"),
        partition::msgid::ID_OUT_OF_RANGE,
        "QVS2017",
    )?);
    ask(
        l,
        AnyRequest::Partition(partition::Request::SetSettings(partition::SetSettings {
            id: 7,
            settings: settings(false, 90),
        })),
    )?;
    let AnyReply::Partition(partition::Reply::GetSettings(got)) = ask(
        l,
        AnyRequest::Partition(partition::Request::GetSettings(partition::GetSettings {
            id: 7,
        })),
    )?
    else {
        return Err("get settings".into());
    };
    if got.delay != 90 || got.panel {
        return Err(format!("its settings now {got:?}"));
    }
    lines.push(refused_as(
        l,
        &AnyRequest::Panel(panel::Request::Function(panel::Function {
            id: 7,
            function: 1,
            args: panel::PanelArgs {
                source: None,
                ipl_type: None,
                keylock: None,
            },
        })),
        "QVS2318",
        "QVS2318",
    )?);
    let AnyReply::Panel(panel::Reply::Functions(f)) = ask(
        l,
        AnyRequest::Panel(panel::Request::Functions(panel::Functions { id: 7 })),
    )?
    else {
        return Err("functions".into());
    };
    let local = partd(&mut q.guest)?
        .panel_functions()
        .map_err(|e| e.to_string())?;
    let (ours, theirs): (Vec<u8>, Vec<u8>) = (
        f.iter().map(|f| f.function).collect(),
        local.iter().map(|f| f.function).collect(),
    );
    if ours != theirs || !f.iter().any(|f| f.function == 3 && f.manual && f.confirm) {
        return Err(format!("the functions {ours:?}, locally {theirs:?}"));
    }
    lines.push(format!(
        "{} panel functions, as the partition manager has them",
        f.len()
    ));

    ask(
        l,
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id: 7,
            profile: profile("NORMAL"),
            revision: None,
        })),
    )?;
    let AnyReply::Partitions(partitions::Reply::Activations(all)) = ask(
        l,
        AnyRequest::Partitions(partitions::Request::Activations(Empty)),
    )?
    else {
        return Err("activations".into());
    };
    let mine = all.iter().find(|a| a.id == 7).ok_or("no activation of 7")?;
    let AnyReply::Profiles(profiles::Reply::Activated(uses)) = ask(
        l,
        AnyRequest::Profiles(profiles::Request::Activated(profiles::Activated { id: 7 })),
    )?
    else {
        return Err("activated".into());
    };
    if mine.since.is_some() || uses.len() != 1 || uses[0].last.is_some() {
        return Err(format!("never activated: {mine:?}, {uses:?}"));
    }
    lines.push("partition 7 never activated: no start, profile NORMAL never used".into());
    Ok(lines)
}

/// Bits 23, 25 and 26: the power-on settings changed; a problem noted,
/// with its severity; the dumps' space.
#[allow(clippy::too_many_lines)] // one family's steps, told in order
fn host_service(q: &mut Qsi<'_>, l: &mut Link) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    ask(
        l,
        AnyRequest::Host(h::Request::SetPowerOn(h::PowerOn {
            start_partitions: false,
            power_off: h::PowerOff::Immediate,
            delay: 120,
        })),
    )?;
    let AnyReply::Host(h::Reply::PowerOn(p)) =
        ask(l, AnyRequest::Host(h::Request::PowerOn(Empty)))?
    else {
        return Err("power on".into());
    };
    let local = partd(&mut q.guest)?.settings().map_err(|e| e.to_string())?;
    if p.start_partitions
        || p.power_off != h::PowerOff::Immediate
        || local.service.host_timeout_secs != 120
        || local.power_on.start_partitions
    {
        return Err(format!(
            "power-on settings {p:?}, locally {:?}",
            local.power_on
        ));
    }
    lines.push("the power-on settings changed, as the partition manager keeps them".into());

    let code = &refcode::CLOCK_NOT_SET;
    channel(&mut q.guest)?.ask(&format!(
        "{} {} by the harness",
        qsvshook::proto::command::POST,
        code.code
    ))?;
    let number = eventually(Duration::from_mins(2), || {
        let Ok(AnyReply::Service(service::Reply::List(page))) = ask(
            l,
            AnyRequest::Service(service::Request::List(service::List {
                filter: service::Filter::default(),
                before: None,
                limit: 20,
            })),
        ) else {
            return None;
        };
        page.problems
            .iter()
            .find(|p| p.refcode.code == code.code)
            .map(|p| p.number)
    })
    .ok_or("no problem opened")?;
    let AnyReply::Service(service::Reply::Note(line)) = ask(
        l,
        AnyRequest::Service(service::Request::Note(service::Note {
            number,
            text: "Set the clock from the firmware".into(),
        })),
    )?
    else {
        return Err("note".into());
    };
    let kept = channel(&mut q.guest)?.text("/data/problems/problems.json")?;
    let AnyReply::Service(service::Reply::GetRated(rated)) = ask(
        l,
        AnyRequest::Service(service::Request::GetRated(service::Get { number })),
    )?
    else {
        return Err("get rated".into());
    };
    if !kept.contains("Set the clock from the firmware")
        || rated.severity != service::Severity::Attention
        || !rated.problem.log.iter().any(|x| x.text == line.text)
    {
        return Err(format!("problem {number}: {rated:?}"));
    }
    lines.push(format!(
        "problem {number} noted `{}`, attention, as the problem log keeps it",
        line.text
    ));

    let AnyReply::Base(base::Reply::DumpSpace(space)) =
        ask(l, AnyRequest::Base(base::Request::DumpSpace(Empty)))?
    else {
        return Err("dump space".into());
    };
    let AnyReply::Dumps(dumps::Reply::Space(each)) = ask(
        l,
        AnyRequest::Dumps(dumps::Request::Space(dumps::GetSpace { id: 0 })),
    )?
    else {
        return Err("dumps space".into());
    };
    let local = partd(&mut q.guest)?
        .dump_space(0)
        .map_err(|e| e.to_string())?
        .map_err(|e| crate::suite::partition::manager::refused(&e))?;
    if space.max_dumps == 0 || each.len() != local.len() {
        return Err(format!("dump space {space:?}, {each:?}, locally {local:?}"));
    }
    lines.push(format!(
        "the host's dumps take {} of {} bytes; {} partitions' dumps, as the partition manager says",
        space.bytes,
        space.limit,
        each.len()
    ));
    Ok(lines)
}

/// The host started again on its disks: what was made over QSI is
/// there, over QSI and at Service Tools.
fn after_restart<'a>(
    run: &Run<'a>,
    q: &mut Qsi<'a>,
    id: &qsitrust::Identity,
    host: qsitrust::Fingerprint,
) -> Result<String, String> {
    power_off(&mut q.guest)?;
    q.guest = crate::suite::install::boot(run, &q.launch)?;
    q.guest.seen(&refcode::QSI_STARTED.tag(), run.deadline())?;
    let mut l = eventually(Duration::from_mins(1), || link(q, id, host).ok())
        .ok_or("the console did not reconnect after the restart")?;
    let AnyReply::Vres(vres::Reply::Pools(pools)) =
        ask(&mut l, AnyRequest::Vres(vres::Request::Pools(Empty)))?
    else {
        return Err("pools".into());
    };
    let AnyReply::Vres(vres::Reply::Switches(switches)) =
        ask(&mut l, AnyRequest::Vres(vres::Request::Switches(Empty)))?
    else {
        return Err("switches".into());
    };
    if !pools.iter().any(|p| p.name == "QSIPOOL") || !switches.iter().any(|s| s.name == "QSISW") {
        return Err(format!("after the restart: {pools:?}, {switches:?}"));
    }
    l.close();
    let deadline = Instant::now() + 3 * run.timeout;
    sign_on(&mut q.guest, USER, PASSWORD, deadline)?;
    option(&mut q.guest, 2, "Work with Disk Units", deadline)?;
    let pools = option(&mut q.guest, 7, "Work with Storage Pools", deadline)?;
    // F3 leaves Service Tools' panels for its main menu.
    q.guest.type_keys(key::F3)?;
    titled(&mut q.guest, "QS/VS Service Tools", deadline)?;
    let switches = crate::suite::network::tools::to_switches(&mut q.guest, deadline)?;
    if !pools.contains("QSIPOOL") || !switches.contains("QSISW") {
        return Err(format!("Service Tools shows:\n{pools}\n{switches}"));
    }
    Ok(
        "after a restart pool QSIPOOL and switch QSISW are there, over QSI and at Service Tools"
            .into(),
    )
}

/// `qsi-additions`.
pub fn additions(run: &Run<'_>) -> Result<String, String> {
    crate::suite::partition::fresh_host(run)?;
    let units = crate::suite::storage::pool_disks(run, 2, crate::suite::storage::POOL_DISK_BYTES)?;
    let mut q = host_disks(run, false, &units)?;
    let deadline = run.deadline();
    let (id, host, mut l) = paired(&mut q, "MC1", deadline)?;
    let mut lines = pools_and_switches(&mut q, &mut l)?;
    lines.extend(partitions_v3(&mut q, &mut l)?);
    lines.extend(host_service(&mut q, &mut l)?);
    l.close();
    lines.push(after_restart(run, &mut q, &id, host)?);
    power_off(&mut q.guest)?;
    Ok(lines.join("; "))
}
