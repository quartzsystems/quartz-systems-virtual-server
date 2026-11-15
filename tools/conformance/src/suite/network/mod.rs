// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 6's suites: virtual switches, partitions' adapters, uplinks.
//! QEMU gives the host further NICs, uplink candidates whose far side is
//! the harness's station (`qemu::wire`): it sees every frame with its
//! tags and answers DHCP, ARP and echo. The cheap guest is QS/VS's own
//! release image, whose `netd` asks for an address by DHCP, or is given
//! a static one and a time server by a seed volume attached as its
//! second unit.
//!
//! The switches are made as the host keeps them: `vswitch.toml` written
//! through the test hooks' channel and the switch service started again,
//! which builds every switch from state; Service Tools makes one in
//! suite 1. What the kernel has is read through the channel too: the
//! links under `/sys/class/net`, each port's counters, the ports' record
//! in the run directory.

mod suites;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use model::EthernetAdapter;
use qsvshook::partition::relayed;
use qsvshook::proto::command;
use vswitch::port::PortSpec;
use vswitch::{Limits, Switch, Switches, Uplink};

use super::base::{Guest, Run, SuiteFn, channel, posted};
use super::partition::manager::{self, Spec};
use super::partition::{QSVS_MEMORY_MIB, fresh_host, host, launch, profile};
use crate::qemu::image::{seed_volume, sparse_copy};
use crate::qemu::session::Session;
use crate::qemu::wire::{Ports, Station};
use crate::qemu::{FIRST_UPLINK_PLACE, Far, UplinkNic};

/// The suites, in the order of the handoff.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("net-switch-create", suites::create),
    ("net-internal", suites::internal),
    ("net-uplink", suites::uplink),
    ("net-tagged", suites::tagged),
    ("net-may-not", suites::may_not),
    ("net-host-unreachable", suites::host_unreachable),
    ("net-management", suites::management),
    ("net-uplink-loss", suites::uplink_loss),
    ("net-qs400", suites::qs400),
    ("net-live", suites::live),
    ("net-service-kill", suites::service_kill),
    ("net-vmm-kill", suites::vmm_kill),
    ("net-not-required", suites::not_required),
    ("net-counters", suites::counters),
    ("net-throughput", suites::throughput),
];

/// The state file of the switches.
const STATE: &str = "/data/state/vswitch.toml";
/// The ports' record.
const PORTS: &str = "/run/vswitchd/ports";

/// The MAC address of uplink NIC `i`.
pub fn uplink_mac(i: usize) -> String {
    format!("52:54:00:aa:00:{:02x}", 0x10 + i)
}

/// A host whose uplink NICs are the station's, its further disks
/// `units`, booted to its partition manager answering.
pub struct Net<'a> {
    /// The host.
    pub guest: Guest<'a>,
    /// The harness's side of the uplinks.
    pub station: Station,
}

/// Boot a host with `nics` uplink NICs to the station (and `user` more
/// on QEMU's user network), with `guests` copies of the cheap guest's
/// disk and `seeds` as their seed volumes (one a guest, or none) as
/// further disks: guest `i`'s disk is `/dev/vd<b + 2i>`, its seed the
/// next.
pub fn boot<'a>(
    run: &Run<'a>,
    nics: usize,
    user: &[Option<(u16, String, u16)>],
    guests: usize,
    seeds: &[Option<String>],
    dhcp: bool,
) -> Result<Net<'a>, String> {
    let image = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    fresh_host(run)?;
    let mut units: Vec<PathBuf> = Vec::new();
    for i in 0..guests {
        let disk = run.file(&format!("guest{i}.img"));
        sparse_copy(image, &disk)?;
        units.push(disk);
        let seed = run.file(&format!("seed{i}.img"));
        let text = seeds
            .get(i)
            .cloned()
            .flatten()
            .unwrap_or_else(|| "schema_version = 1\n".into());
        seed_volume(&seed, release::SEED_LABEL, &text)?;
        units.push(seed);
    }
    let ports = Station::ports(nics).map_err(|e| e.to_string())?;
    let station = Station::start(&ports, dhcp).map_err(|e| e.to_string())?;
    let mut l = launch(run, &units)?;
    l.uplinks = ports
        .iter()
        .enumerate()
        .map(|(i, p)| UplinkNic {
            mac: uplink_mac(i),
            far: Far::Station(*p),
        })
        .chain(user.iter().enumerate().map(|(j, fwd)| UplinkNic {
            mac: uplink_mac(nics + j),
            far: Far::User(fwd.clone()),
        }))
        .collect();
    let guest = host(run, &l)?;
    let _ = ports_of(&ports);
    Ok(Net { guest, station })
}

fn ports_of(p: &[Ports]) -> usize {
    p.len()
}

/// The disk of guest `i` as the host has it.
pub fn guest_disk(i: usize) -> String {
    let letter = char::from(b'b' + u8::try_from(2 * i).unwrap_or(0));
    format!("/dev/vd{letter}")
}

/// Its seed volume.
pub fn guest_seed(i: usize) -> String {
    let letter = char::from(b'b' + u8::try_from(2 * i + 1).unwrap_or(0));
    format!("/dev/vd{letter}")
}

/// A seed of a static address `address` (with its prefix) and the time
/// server `ntp`, which the guest's network service then asks.
pub fn static_seed(name: &str, address: &str, ntp: &str) -> String {
    format!(
        "schema_version = 1\n\n[host]\nname = \"{name}\"\nntp_servers = [\"{ntp}\"]\n\n[network]\nmode = \"static\"\naddress = \"{address}\"\n"
    )
}

/// The location code of the interface whose MAC address is `mac`, from
/// the inventory PID 1 keeps.
pub fn location_of(guest: &mut Session, mac: &str) -> Result<String, String> {
    let text = channel(guest)?.text("/data/state/inventory.toml")?;
    let inventory: state::Inventory =
        state::store::parse(&text).map_err(|e| format!("inventory.toml: {e}"))?;
    inventory
        .interfaces
        .into_iter()
        .find(|i| i.mac.eq_ignore_ascii_case(mac))
        .map(|i| i.location)
        .ok_or_else(|| format!("no interface {mac} in the inventory"))
}

/// The location of uplink NIC `i`.
pub fn uplink_location(guest: &mut Session, i: usize) -> Result<String, String> {
    location_of(guest, &uplink_mac(i))
}

/// A switch.
pub fn switch(name: &str, id: u32, vlans: &[u16], mtu: u32, uplink: Option<Uplink>) -> Switch {
    Switch {
        name: name.into(),
        id,
        vlans: vlans.to_vec(),
        mtu,
        uplink,
    }
}

/// The links of the host.
pub fn links(guest: &mut Session) -> Result<Vec<String>, String> {
    channel(guest)?.ls("/sys/class/net")
}

/// Keep `switches` as the host's and start the switch service again,
/// which builds them; wait until each bridge is there and the service
/// is ready.
pub fn set_switches(guest: &mut Session, switches: &[Switch]) -> Result<(), String> {
    let state = Switches {
        limits: Limits::default(),
        switches: switches.to_vec(),
    };
    let text = state::store::render(&state).map_err(|e| e.to_string())?;
    let line = format!("{} {STATE} {}", command::PUT, hex::encode(text.as_bytes()));
    channel(guest)?.ask(&line)?;
    restart_service(guest)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    for s in switches {
        let bridge = s.bridge();
        loop {
            if links(guest)?.contains(&bridge) {
                break;
            }
            if Instant::now() > deadline {
                return Err(format!("{bridge} was not made for switch {}", s.name));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    Ok(())
}

/// Kill the switch service and wait until PID 1 has it ready again.
pub fn restart_service(guest: &mut Session) -> Result<u32, String> {
    let before = service_pid(guest)?;
    channel(guest)?.ask(&format!("{} {}", command::KILL, vswitchd::NAME))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = channel(guest)?.status()?;
        if let Some(s) = status.service(vswitchd::NAME)
            && s.pid != before
            && s.pid != 0
            && s.state == "ready"
        {
            return Ok(s.pid);
        }
        if Instant::now() > deadline {
            return Err("the switch service was not ready again within 60 s".into());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// The switch service's process.
pub fn service_pid(guest: &mut Session) -> Result<u32, String> {
    let status = channel(guest)?.status()?;
    status
        .service(vswitchd::NAME)
        .map(|s| s.pid)
        .ok_or_else(|| "the switch service is not in PID 1's table".into())
}

/// The ports the switch service has, as its record says.
pub fn ports(guest: &mut Session) -> Result<Vec<PortSpec>, String> {
    let bytes = channel(guest)?.cat(PORTS).unwrap_or_default();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    ipc::decode(&bytes).map_err(|e| format!("{PORTS}: {e}"))
}

/// A counter of a link: `rx_packets`, `tx_bytes`...
pub fn counter(guest: &mut Session, link: &str, name: &str) -> Result<u64, String> {
    let text = channel(guest)?.text(&format!("/sys/class/net/{link}/statistics/{name}"))?;
    text.trim()
        .parse()
        .map_err(|_| format!("{link} {name}: {text:?}"))
}

/// An adapter of the cheap guest's profile.
pub fn adapter(slot: u32, switch: &str, vlan: u32, tagged: &[u32]) -> EthernetAdapter {
    EthernetAdapter {
        slot,
        switch: Some(switch.into()),
        vlan,
        tagged: tagged.to_vec(),
        mac: None,
        required: true,
    }
}

/// A profile of the cheap guest `i`, with its seed as its second unit,
/// and `adapters`.
pub fn guest_spec(i: usize, adapters: Vec<EthernetAdapter>) -> Spec {
    let mut spec = profile(1, QSVS_MEMORY_MIB, "side a", &guest_disk(i));
    spec.backing.push(guest_seed(i));
    spec.ethernet = adapters;
    spec
}

/// Define partition `name` with `spec` and activate it; its ID.
pub fn activate(guest: &mut Session, name: &str, spec: &Spec) -> Result<u32, String> {
    let id = manager::partition(guest, name)?;
    manager::activate(guest, id, spec).map_err(|e| manager::refused(&e))?;
    Ok(id)
}

/// The MAC address of slot `slot` of partition `id`, as its profile
/// keeps it.
pub fn mac_of(guest: &mut Session, id: u32, slot: u32) -> Result<[u8; 6], String> {
    use partd::Calls as _;
    let all = manager::partd(guest)?
        .adapters(id)
        .map_err(|e| e.to_string())?;
    all.iter()
        .find(|a| a.slot == slot)
        .and_then(|a| vswitch::mac::parse(&a.mac))
        .ok_or_else(|| format!("partition {id} has no adapter in slot {slot}"))
}

/// Wait for partition `name`'s own network service to say it has an
/// address; the address.
pub fn guest_address(
    guest: &mut Session,
    name: &str,
    deadline: Instant,
) -> Result<Ipv4Addr, String> {
    guest.expect(
        &relayed(name, &posted(&refcode::ADDRESS_ACQUIRED, "")),
        deadline,
    )?;
    let rest = guest.rest_of_line(deadline)?;
    rest.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|w| w.parse::<Ipv4Addr>().ok())
        .ok_or_else(|| format!("no address in `{rest}`"))
}

/// Wait for partition `name`'s base to be ready.
pub fn guest_ready(guest: &mut Session, name: &str, deadline: Instant) -> Result<(), String> {
    guest.expect(&relayed(name, &posted(&refcode::BASE_READY, "")), deadline)
}

/// The tap of slot `slot` of partition `id`.
pub fn tap(id: u32, slot: u32) -> String {
    vswitch::switch::tap_name(id, slot)
}

/// The uplink's place on the PCI bus, for the record.
pub fn uplink_place(i: usize) -> u8 {
    FIRST_UPLINK_PLACE + u8::try_from(i).unwrap_or(0)
}
