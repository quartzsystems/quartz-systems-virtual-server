// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host family: the resources and settings (the partition
//! manager's), and the hardware: the inventory, each item's owner from
//! the pool service's disks and the switch service's interfaces.

use qsi::any::AnyReply;
use qsi::host::host::{self, Amount, Hardware, Item, ItemKind, Owner, ProcPool, Reply, Request};
use qsisess::Note;

use super::{Services, reached};

const PARTD: &str = "partition manager";
const POOLD: &str = "pool service";
const VSWITCHD: &str = "switch service";

type Answered = Result<AnyReply, Vec<Note>>;

fn amount(a: &resource::Amount) -> Amount {
    Amount {
        installed: a.installed,
        configurable: a.configurable,
        available: a.available,
    }
}

/// The host's resources, as the partition manager accounts them.
pub fn resources(
    r: &resource::HostResources,
    settings: &resource::Settings,
    partitions: u32,
) -> host::Host {
    host::Host {
        sockets: r.sockets,
        cores: r.cores,
        threads_per_core: r.threads_per_core,
        processors: amount(&r.processors),
        memory: amount(&r.memory),
        memory_block_mib: r.memory_block_mib,
        reserved_cores: r.reserved_cores.clone(),
        reserved_memory_mib: r.memory.installed.saturating_sub(r.memory.configurable),
        pools: r
            .pools
            .iter()
            .map(|p| ProcPool {
                name: p.name.clone(),
                cores: p.cores.clone(),
                maximum: p.maximum.0,
                entitled: p.entitled.0,
                available: p.available.0,
                partitions: p.partitions,
            })
            .collect(),
        partitions,
        max_partitions: settings.max_partitions,
    }
}

fn settings(s: &resource::Settings) -> host::Settings {
    host::Settings {
        max_partitions: s.max_partitions,
        memory_block_mib: s.memory_block_mib,
        reserved_cores: s.reserved_cores,
        reserved_memory_mib: s.reserved_memory_mib,
        mac_prefix: s.mac_prefix.clone(),
        delayed_grace_secs: s.service.delayed_grace_secs,
        system_timeout_secs: s.service.system_timeout_secs,
        host_timeout_secs: s.service.host_timeout_secs,
        channel_wait_secs: s.service.channel_wait_secs,
        max_dumps: s.service.max_dumps,
        max_dump_mib: s.service.max_dump_mib,
    }
}

fn item(kind: ItemKind, location: &str, name: &str) -> Item {
    Item {
        location: location.to_string(),
        kind,
        name: name.to_string(),
        description: String::new(),
        serial: String::new(),
        bytes: 0,
        mac: String::new(),
        speed_mbps: 0,
        carrier: false,
        owner: Owner::None,
        owner_name: String::new(),
        partition: 0,
        what: String::new(),
    }
}

/// The hardware: the inventory's items, each with its owner.
pub fn hardware(
    inventory: &state::Inventory,
    disks: &[poold::Disk],
    interfaces: &[vswitchd::Interface],
) -> Hardware {
    let mut items = Vec::new();
    for d in &inventory.disks {
        let mut i = item(ItemKind::Disk, &d.location, &d.name);
        i.description.clone_from(&d.model);
        i.serial.clone_from(&d.serial);
        i.bytes = d.bytes;
        if let Some(seen) = disks.iter().find(|s| s.name == d.name) {
            i.what.clone_from(&seen.what);
            (i.owner, i.owner_name) = if !seen.pool.is_empty() {
                (Owner::Pool, seen.pool.clone())
            } else if seen.foreign {
                (Owner::Foreign, String::new())
            } else if seen.free {
                (Owner::None, String::new())
            } else {
                (Owner::Host, String::new())
            };
        }
        items.push(i);
    }
    for n in &inventory.interfaces {
        let mut i = item(ItemKind::Interface, &n.location, &n.name);
        i.description.clone_from(&n.driver);
        i.mac.clone_from(&n.mac);
        i.speed_mbps = n.speed_mbps.unwrap_or(0);
        i.carrier = n.carrier;
        if let Some(seen) = interfaces.iter().find(|s| s.name == n.name) {
            i.what.clone_from(&seen.what);
            i.carrier = seen.carrier;
            (i.owner, i.owner_name) = if seen.management {
                (Owner::Host, String::new())
            } else if !seen.switch.is_empty() {
                (Owner::Switch, seen.switch.clone())
            } else {
                (Owner::None, String::new())
            };
        }
        items.push(i);
    }
    for p in &inventory.pci {
        let mut i = item(ItemKind::Pci, &p.location, &p.address);
        i.description = format!("{} {} {}", p.vendor, p.device, p.class);
        i.what = p.driver.clone().unwrap_or_default();
        items.push(i);
    }
    Hardware {
        manufacturer: inventory.system.manufacturer.clone(),
        product: inventory.system.product.clone(),
        processor: inventory.processors.model.clone(),
        virtualization: inventory.processors.virtualization.clone(),
        items,
    }
}

/// Answer a host request.
pub(super) fn answer(s: &mut Services, r: &Request) -> Answered {
    Ok(AnyReply::Host(match r {
        Request::Host(_) => {
            let r = reached(PARTD, s.partd.resources())?;
            let set = reached(PARTD, s.partd.settings())?;
            let partitions = reached(PARTD, s.partd.list())?.len();
            Reply::Host(resources(
                &r,
                &set,
                u32::try_from(partitions).unwrap_or(u32::MAX),
            ))
        }
        Request::Settings(_) => Reply::Settings(settings(&reached(PARTD, s.partd.settings())?)),
        Request::Hardware(_) => {
            let inventory = s.facts.inventory()?;
            let disks = reached(POOLD, s.poold.disks())?;
            let interfaces = reached(VSWITCHD, s.vswitchd.interfaces())?;
            Reply::Hardware(hardware(&inventory, &disks, &interfaces))
        }
        Request::Devices(_) => Reply::Devices(
            reached(PARTD, s.partd.devices())?
                .iter()
                .filter(|d| d.present)
                .map(super::convert::device)
                .collect(),
        ),
        Request::PowerOn(_) | Request::SetPowerOn(_) => {
            return Err(vec![super::base::not_agreed()]);
        }
    }))
}
