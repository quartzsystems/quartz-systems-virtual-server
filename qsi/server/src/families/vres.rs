// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The vres family: storage pools, virtual disks and the media library
//! (the pool service's), the drives of a partition and the processor
//! pools (the partition manager's), the switches (the switch service's).

use qsi::Empty;
use qsi::any::AnyReply;
use qsi::host::model::Holder;
use qsi::host::vres::{self, Reply, Request};
use qsisess::Note;

use super::{Services, local, reached};

const POOLD: &str = "pool service";
const VSWITCHD: &str = "switch service";
const PARTD: &str = "partition manager";

type Answered = Result<AnyReply, Vec<Note>>;

fn holder(h: &poold::Holder) -> Holder {
    Holder {
        partition: h.partition,
        slot: h.slot,
    }
}

/// A storage pool, without its mount point: that is the host's alone.
pub fn pool(p: &poold::PoolStatus) -> vres::StoragePool {
    vres::StoragePool {
        name: p.name.clone(),
        uuid: p.uuid.clone(),
        protection: p.protection.clone(),
        state: p.state.clone(),
        work: p.work.clone(),
        work_unit: p.work_unit,
        percent: p.percent,
        bytes: p.bytes,
        allocated: p.allocated,
        provisioned: p.provisioned,
        repository: p.repository,
        units: p
            .units
            .iter()
            .map(|u| vres::PoolUnit {
                number: u.number,
                disk: u.disk.clone(),
                serial: u.serial.clone(),
                location: u.location.clone(),
                bytes: u.bytes,
                state: u.state.clone(),
            })
            .collect(),
        failure: p.failure.clone(),
    }
}

/// A virtual disk.
pub fn vdisk(v: &poold::Vdisk) -> vres::Vdisk {
    vres::Vdisk {
        pool: v.pool.clone(),
        name: v.name.clone(),
        bytes: v.bytes,
        allocated: v.allocated,
        thick: v.thick,
        holder: holder(&v.holder),
    }
}

/// An optical image.
pub fn image(i: &poold::Image) -> vres::Image {
    vres::Image {
        name: i.name.clone(),
        bytes: i.bytes,
        label: i.label.clone(),
        holder: holder(&i.holder),
    }
}

/// A tape volume.
pub fn volume(v: &poold::Volume) -> vres::Volume {
    vres::Volume {
        name: v.name.clone(),
        identifier: v.identifier.clone(),
        capacity_mib: v.capacity_mib,
        used: v.used,
        protected: v.protected,
        holder: holder(&v.holder),
    }
}

/// A switch, without its bridge: that is the host's alone.
pub fn switch(s: &vswitchd::SwitchStatus) -> Option<vres::Switch> {
    let sw = s.switch.as_ref()?;
    Some(vres::Switch {
        name: sw.name.clone(),
        id: sw.id,
        vlans: sw.vlans.clone(),
        mtu: sw.mtu,
        uplink: sw.uplink.as_ref().map(|u| vres::Uplink {
            interfaces: u.interfaces.clone(),
            bond: u.bond.map(|b| match b {
                vswitch::Bond::ActiveBackup => vres::Bond::ActiveBackup,
                vswitch::Bond::Lacp => vres::Bond::Lacp,
            }),
            native_vlan: u.native_vlan,
        }),
        uplink_state: s.uplink_state.clone(),
        members: s
            .members
            .iter()
            .map(|m| vres::Member {
                location: m.location.clone(),
                name: m.name.clone(),
                carrier: m.carrier,
                speed_mbps: m.speed_mbps,
            })
            .collect(),
        ports: s.ports,
        failure: s.failure.clone(),
    })
}

/// A partition's port, without its tap: that is the host's alone.
pub fn port(p: &vswitchd::PortStatus) -> vres::Port {
    let c = &p.counters;
    vres::Port {
        partition: p.spec.partition,
        slot: p.spec.slot,
        switch: p.spec.switch.clone(),
        vlan: p.spec.vlan,
        tagged: p.spec.tagged.clone(),
        mac: vswitch::mac::text(&p.spec.mac),
        link: p.link,
        counters: vres::Counters {
            sent_bytes: c.sent_bytes,
            sent_packets: c.sent_packets,
            sent_dropped: c.sent_dropped,
            received_bytes: c.received_bytes,
            received_packets: c.received_packets,
            received_dropped: c.received_dropped,
        },
    }
}

/// Where a virtual disk is named: every unit of every profile whose
/// backing store it is. partd's own answer is text, so it is read from
/// the profiles.
fn named(s: &mut Services, pool: &str, name: &str) -> Result<Vec<vres::Naming>, Vec<Note>> {
    let wanted = format!("{pool}/{name}");
    let mut out = Vec::new();
    for p in reached(PARTD, s.partd.list())? {
        for profile_name in local(PARTD, s.partd.profiles(p.id))? {
            let profile = local(PARTD, s.partd.profile(p.id, profile_name))?;
            for adapter in &profile.scsi {
                for lun in &adapter.luns {
                    if lun.backing == wanted {
                        out.push(vres::Naming {
                            partition: p.id,
                            name: p.name.clone(),
                            profile: profile.name.clone(),
                            slot: adapter.slot,
                            lun: lun.lun,
                        });
                    }
                }
            }
        }
    }
    Ok(out)
}

fn drive(m: &partd::proto::MediaSlot) -> vres::Drive {
    vres::Drive {
        slot: m.slot,
        kind: match m.kind.as_str() {
            "tape" => vres::DriveKind::Tape,
            "drive" => vres::DriveKind::PhysicalTape,
            _ => vres::DriveKind::Optical,
        },
        name: m.name.clone(),
        prevented: m.prevented,
    }
}

/// A physical tape drive as QSI has it.
pub fn tape_drive(d: &partd::proto::TapeDrive) -> vres::TapeDrive {
    vres::TapeDrive {
        location: d.location.clone(),
        vendor: d.vendor.clone(),
        product: d.product.clone(),
        serial: d.serial.clone(),
        released: d.released,
        loaded: d.loaded,
        holder: Holder {
            partition: d.holder,
            slot: d.slot,
        },
    }
}

fn proc_pool(spec: &vres::ProcPoolSpec) -> resource::Pool {
    resource::Pool {
        name: spec.name.clone(),
        cores: spec.cores.clone(),
        maximum_units: spec.maximum_units.map(model::Units),
    }
}

/// A request given to a group that is not its own.
fn not_here() -> Vec<Note> {
    vec![Note {
        msgid: qsi::msgid::UNKNOWN.into(),
        text: "This host does not answer that request.".into(),
        help: String::new(),
    }]
}

/// Answer a vres request. An image's upload is the connection's.
pub(super) fn answer(s: &mut Services, r: Request) -> Answered {
    let reply = match &r {
        Request::Pools(_)
        | Request::Vdisks(_)
        | Request::VdiskCreate(_)
        | Request::VdiskResize(_)
        | Request::VdiskDelete(_)
        | Request::VdiskNamed(_) => storage(s, r),
        Request::Images(_)
        | Request::ImageRename(_)
        | Request::ImageDelete(_)
        | Request::Drives(_)
        | Request::Load(_)
        | Request::Unload(_)
        | Request::Volumes(_)
        | Request::VolumeCreate(_)
        | Request::VolumeDelete(_)
        | Request::VolumeProtect(_) => library(s, r),
        Request::Switches(_)
        | Request::Ports(_)
        | Request::Fdb(_)
        | Request::Vlans(_)
        | Request::Limits(_) => network(s, r),
        Request::ProcPoolCreate(_) | Request::ProcPoolChange(_) | Request::ProcPoolDelete(_) => {
            processors(s, r)
        }
        // A stream's: the connection opens it.
        Request::ImageUpload(_) => Err(not_here()),
        Request::Disks(_)
        | Request::PoolCreate(_)
        | Request::PoolDelete(_)
        | Request::PoolRename(_)
        | Request::UnitReplace(_)
        | Request::SpareAdd(_)
        | Request::SpareRemove(_)
        | Request::Designate(_)
        | Request::PoolExtend(_) => managed_pools(s, r),
        Request::SwitchCreate(_)
        | Request::SwitchChange(_)
        | Request::SwitchRename(_)
        | Request::SwitchDelete(_)
        | Request::UplinkSet(_)
        | Request::UplinkClear(_) => managed_switches(s, r),
        Request::TapeDrives(_) => reached(PARTD, s.partd.tape_drives()).map(|drives| {
            Reply::TapeDrives(
                drives
                    .iter()
                    .filter(|d| d.present)
                    .map(tape_drive)
                    .collect(),
            )
        }),
    }?;
    Ok(AnyReply::Vres(reply))
}

/// Storage pools and virtual disks.
fn storage(s: &mut Services, r: Request) -> Result<Reply, Vec<Note>> {
    Ok(match r {
        Request::Pools(_) => {
            Reply::Pools(reached(POOLD, s.poold.pools())?.iter().map(pool).collect())
        }
        Request::Vdisks(v) => Reply::Vdisks(
            reached(POOLD, s.poold.vdisks(v.pool))?
                .iter()
                .map(vdisk)
                .collect(),
        ),
        Request::VdiskCreate(v) => {
            let made = local(POOLD, s.poold.create_vdisk(v.pool, v.name, v.mib, v.thick))?;
            Reply::VdiskCreate(vdisk(&made))
        }
        Request::VdiskResize(v) => {
            let grown = local(POOLD, s.poold.resize_vdisk(v.pool, v.name, v.mib))?;
            Reply::VdiskResize(vdisk(&grown))
        }
        Request::VdiskDelete(v) => {
            local(POOLD, s.poold.delete_vdisk(v.pool, v.name, v.force))?;
            Reply::VdiskDelete(Empty)
        }
        Request::VdiskNamed(v) => Reply::VdiskNamed(named(s, &v.pool, &v.name)?),
        _ => return Err(not_here()),
    })
}

/// The media library and the drives.
fn library(s: &mut Services, r: Request) -> Result<Reply, Vec<Note>> {
    Ok(match r {
        Request::Images(_) => Reply::Images(
            reached(POOLD, s.poold.images())?
                .iter()
                .map(image)
                .collect(),
        ),
        Request::ImageRename(i) => {
            local(POOLD, s.poold.rename_image(i.name, i.to))?;
            Reply::ImageRename(Empty)
        }
        Request::ImageDelete(i) => {
            local(POOLD, s.poold.delete_image(i.name))?;
            Reply::ImageDelete(Empty)
        }
        Request::Drives(d) => Reply::Drives(
            local(PARTD, s.partd.media(d.id))?
                .iter()
                .map(drive)
                .collect(),
        ),
        Request::Load(l) => {
            local(PARTD, s.partd.load_media(l.id, l.slot, l.name))?;
            Reply::Load(Empty)
        }
        Request::Unload(u) => Reply::Unload(vres::Unloaded {
            name: local(PARTD, s.partd.unload_media(u.id, u.slot, u.forced))?,
        }),
        Request::Volumes(_) => Reply::Volumes(
            reached(POOLD, s.poold.volumes())?
                .iter()
                .map(volume)
                .collect(),
        ),
        Request::VolumeCreate(v) => {
            let made = local(POOLD, s.poold.create_volume(v.name, v.identifier, v.mib))?;
            Reply::VolumeCreate(volume(&made))
        }
        Request::VolumeDelete(v) => {
            local(POOLD, s.poold.delete_volume(v.name))?;
            Reply::VolumeDelete(Empty)
        }
        Request::VolumeProtect(v) => {
            local(POOLD, s.poold.protect_volume(v.name, v.on))?;
            Reply::VolumeProtect(Empty)
        }
        _ => return Err(not_here()),
    })
}

/// The switches.
fn network(s: &mut Services, r: Request) -> Result<Reply, Vec<Note>> {
    Ok(match r {
        Request::Switches(_) => Reply::Switches(
            reached(VSWITCHD, s.vswitchd.switches())?
                .iter()
                .filter_map(switch)
                .collect(),
        ),
        Request::Ports(p) => Reply::Ports(
            reached(VSWITCHD, s.vswitchd.ports(p.switch))?
                .iter()
                .map(port)
                .collect(),
        ),
        Request::Fdb(f) => Reply::Fdb(
            local(VSWITCHD, s.vswitchd.fdb(f.switch))?
                .into_iter()
                .map(|r| vres::FdbRow {
                    mac: r.mac,
                    vlan: r.vlan,
                    port: r.port,
                    partition: r.partition,
                    slot: r.slot,
                    origin: r.origin,
                })
                .collect(),
        ),
        Request::Vlans(_) => Reply::Vlans(
            reached(VSWITCHD, s.vswitchd.vlans())?
                .into_iter()
                .map(|v| vres::VlanUse {
                    switch: v.switch,
                    vlan: v.vlan,
                    native: v.native,
                    ports: v
                        .ports
                        .into_iter()
                        .map(|(partition, slot, tagged)| vres::VlanPort {
                            partition,
                            slot,
                            tagged,
                        })
                        .collect(),
                })
                .collect(),
        ),
        Request::Limits(_) => {
            let l = reached(VSWITCHD, s.vswitchd.limits())?;
            Reply::Limits(vres::Limits {
                switches: l.switches,
                vlans_per_switch: l.vlans_per_switch,
                adapters_per_partition: l.adapters_per_partition,
                ports_per_switch: l.ports_per_switch,
            })
        }
        _ => return Err(not_here()),
    })
}

/// A disk a pool may take, as QSI has it.
pub fn pool_disk(d: &poold::Disk) -> vres::PoolDisk {
    vres::PoolDisk {
        name: d.name.clone(),
        serial: d.serial.clone(),
        location: d.location.clone(),
        bytes: d.bytes,
        block_size: d.logical_block,
        what: d.what.clone(),
        pool: d.pool.clone(),
        free: d.free,
        foreign: d.foreign,
    }
}

/// A protection as the pool service names it.
fn protection(p: vres::Protection) -> Result<&'static str, Vec<Note>> {
    Ok(match p {
        vres::Protection::None => "none",
        vres::Protection::Mirrored => "mirrored",
        vres::Protection::Raid5 => "raid5",
        vres::Protection::Raid6 => "raid6",
        vres::Protection::Other(_) => return Err(vec![super::convert::no_such("protection")]),
    })
}

/// QSI's identifier of a pool service's refusal (bit 20).
fn pools_identifier(id: &str) -> Option<&'static str> {
    use vres::pools as p;
    Some(match id {
        "QVS3001" => p::BAD_NAME,
        "QVS3002" => p::NAME_TAKEN,
        "QVS3003" => p::NO_POOL,
        "QVS3004" => p::DISK_COUNT,
        "QVS3005" => p::NO_DISK,
        "QVS3006" => p::DISK_TAKEN,
        "QVS3007" => p::FOREIGN,
        "QVS3008" => p::TOO_SMALL,
        "QVS3009" => p::BLOCK_SIZES,
        "QVS3010" | "QVS3017" => p::SPARES,
        "QVS3011" => p::TOO_LARGE,
        "QVS3012" => p::NOT_EMPTY,
        "QVS3013" => p::HELD,
        "QVS3014" => p::NOT_USABLE,
        "QVS3015" => p::NOT_FAILED,
        "QVS3016" => p::STEP_FAILED,
        "QVS3019" => p::CANNOT_GROW,
        "QVS3205" => p::MEDIA_IN_USE,
        _ => return None,
    })
}

/// QSI's identifier of a switch service's refusal (bit 19).
fn switches_identifier(id: &str) -> Option<&'static str> {
    use vres::switches as w;
    Some(match id {
        "QVS4001" => w::BAD_NAME,
        "QVS4002" => w::NAME_TAKEN,
        "QVS4003" => w::NO_SWITCH,
        "QVS4004" | "QVS4005" => w::BAD_VLAN,
        "QVS4006" | "QVS4105" => w::BAD_MTU,
        "QVS4007" => w::TOO_MANY,
        "QVS4008" | "QVS4009" => w::NAMED,
        "QVS4010" => w::NATIVE_NOT_CARRIED,
        "QVS4012" => w::NOT_BUILT,
        "QVS4015" => w::PORTS_ON,
        "QVS4101" => w::NO_INTERFACE,
        "QVS4102" => w::MANAGEMENT,
        "QVS4103" => w::OTHER_SWITCH,
        "QVS4104" => w::NO_BOND,
        "QVS4106" | "QVS4107" => w::UPLINK_SIZE,
        _ => return None,
    })
}

/// A refusal led by QSI's identifier from `table`, the host's after it.
fn led_by(notes: Vec<Note>, table: fn(&str) -> Option<&'static str>) -> Vec<Note> {
    let Some(ours) = notes.first().and_then(|n| table(&n.msgid)) else {
        return notes;
    };
    let mut all = vec![Note {
        msgid: ours.into(),
        text: notes[0].text.clone(),
        help: notes[0].help.clone(),
    }];
    all.extend(notes);
    all
}

/// Storage pools made and changed (bit 20): the pool service's.
fn managed_pools(s: &mut Services, r: Request) -> Result<Reply, Vec<Note>> {
    let lead = |n| led_by(n, pools_identifier);
    Ok(match r {
        Request::Disks(_) => Reply::Disks(
            reached(POOLD, s.poold.candidates())?
                .iter()
                .map(pool_disk)
                .collect(),
        ),
        Request::PoolCreate(c) => {
            let p = protection(c.protection)?.to_string();
            let made = local(
                POOLD,
                s.poold.create_pool(c.name, p, c.disks, c.spares, c.confirm),
            )
            .map_err(lead)?;
            Reply::PoolCreate(pool(&made))
        }
        Request::PoolDelete(d) => {
            local(POOLD, s.poold.delete_pool(d.name)).map_err(lead)?;
            Reply::PoolDelete(Empty)
        }
        Request::PoolRename(p) => {
            local(POOLD, s.poold.rename_pool(p.name, p.to)).map_err(lead)?;
            Reply::PoolRename(Empty)
        }
        Request::UnitReplace(u) => {
            local(
                POOLD,
                s.poold.replace_unit(u.pool, u.unit, u.disk, u.confirm),
            )
            .map_err(lead)?;
            Reply::UnitReplace(Empty)
        }
        Request::SpareAdd(a) => {
            local(POOLD, s.poold.add_spare(a.pool, a.disk, a.confirm)).map_err(lead)?;
            Reply::SpareAdd(Empty)
        }
        Request::SpareRemove(a) => {
            local(POOLD, s.poold.remove_spare(a.pool, a.disk)).map_err(lead)?;
            Reply::SpareRemove(Empty)
        }
        Request::Designate(d) => {
            local(POOLD, s.poold.designate(d.pool)).map_err(lead)?;
            Reply::Designate(Empty)
        }
        // A pool's set has a fixed size: one that is there cannot grow.
        Request::PoolExtend(e) => {
            let there = reached(POOLD, s.poold.pools())?
                .iter()
                .any(|p| p.name == e.pool);
            let why = if there {
                qvsmsg::SPOOL_FIXED_SIZE.with(&[&e.pool])
            } else {
                qvsmsg::SPOOL_MISSING.with(&[&e.pool])
            };
            return Err(lead(super::convert::notes(&[why])));
        }
        _ => return Err(not_here()),
    })
}

/// An uplink as the switch service has it.
fn uplink(u: vres::Uplink) -> Result<vswitch::Uplink, Vec<Note>> {
    let bond = match u.bond {
        None => None,
        Some(vres::Bond::ActiveBackup) => Some(vswitch::Bond::ActiveBackup),
        Some(vres::Bond::Lacp) => Some(vswitch::Bond::Lacp),
        Some(vres::Bond::Other(_)) => return Err(vec![super::convert::no_such("bond mode")]),
    };
    Ok(vswitch::Uplink {
        interfaces: u.interfaces,
        bond,
        native_vlan: u.native_vlan,
    })
}

/// Switches made and changed (bit 19): the switch service's. The
/// management interface is refused as an uplink there (`QVS4102`).
fn managed_switches(s: &mut Services, r: Request) -> Result<Reply, Vec<Note>> {
    let lead = |n| led_by(n, switches_identifier);
    Ok(match r {
        Request::SwitchCreate(w) => {
            let made = local(VSWITCHD, s.vswitchd.create(w.name, w.vlans, w.mtu)).map_err(lead)?;
            Reply::SwitchCreate(switch(&made).ok_or_else(not_here)?)
        }
        Request::SwitchChange(w) => {
            local(VSWITCHD, s.vswitchd.change(w.name, w.vlans, w.mtu)).map_err(lead)?;
            Reply::SwitchChange(Empty)
        }
        Request::SwitchRename(w) => {
            local(VSWITCHD, s.vswitchd.rename(w.name, w.to)).map_err(lead)?;
            Reply::SwitchRename(Empty)
        }
        Request::SwitchDelete(w) => {
            local(VSWITCHD, s.vswitchd.delete(w.name, w.force)).map_err(lead)?;
            Reply::SwitchDelete(Empty)
        }
        Request::UplinkSet(u) => {
            let up = uplink(u.uplink)?;
            local(VSWITCHD, s.vswitchd.set_uplink(u.switch, up)).map_err(lead)?;
            Reply::UplinkSet(Empty)
        }
        Request::UplinkClear(u) => {
            local(VSWITCHD, s.vswitchd.clear_uplink(u.switch)).map_err(lead)?;
            Reply::UplinkClear(Empty)
        }
        _ => return Err(not_here()),
    })
}

/// The processor pools.
fn processors(s: &mut Services, r: Request) -> Result<Reply, Vec<Note>> {
    Ok(match r {
        Request::ProcPoolCreate(spec) => {
            local(PARTD, s.partd.create_pool(proc_pool(&spec)))?;
            Reply::ProcPoolCreate(Empty)
        }
        Request::ProcPoolChange(spec) => {
            local(PARTD, s.partd.change_pool(proc_pool(&spec)))?;
            Reply::ProcPoolChange(Empty)
        }
        Request::ProcPoolDelete(d) => {
            local(PARTD, s.partd.delete_pool(d.name))?;
            Reply::ProcPoolDelete(Empty)
        }
        _ => return Err(not_here()),
    })
}
