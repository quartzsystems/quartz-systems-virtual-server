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

mod qs400;
pub(crate) mod suites;
pub(crate) mod tools;

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
use crate::qemu::wire::Station;
use crate::qemu::{Drive, FIRST_UPLINK_PLACE, Far, Launch, UplinkNic};

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

/// The host's management NIC, on QEMU's user network.
pub const MANAGEMENT_MAC: &str = "52:54:00:12:34:56";

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

/// What a suite's host has.
#[derive(Clone, Debug, Default)]
pub struct Rig {
    /// Uplink NICs to the station, each with its largest MTU or none.
    pub nics: Vec<Option<u32>>,
    /// More on QEMU's user network, each with a port forwarded or none.
    pub user: Vec<Option<(u16, String, u16)>>,
    /// Copies of the cheap guest's disk.
    pub guests: usize,
    /// Their seeds, by guest; none gives an empty one.
    pub seeds: Vec<Option<String>>,
    /// Whether the station answers DHCP.
    pub dhcp: bool,
    /// Further disks after the guests', as they are.
    pub disks: Vec<PathBuf>,
}

impl Rig {
    /// `nics` plain uplink NICs and `guests` cheap guests, with DHCP.
    pub fn new(nics: usize, guests: usize) -> Self {
        Self {
            nics: vec![None; nics],
            guests,
            dhcp: true,
            ..Self::default()
        }
    }
}

/// The launch of a host for `rig`, and the station on its uplinks. The
/// host's seed is `/dev/vdb`; the cheap guest `i`'s disk is
/// `/dev/vd<c + 2i>`, its seed the next.
pub fn rig_launch(run: &Run<'_>, rig: &Rig, base: Launch) -> Result<(Launch, Station), String> {
    let image = run
        .release_disk
        .ok_or("the release image is not built; run `all` first")?;
    // The host's own seed first: an unconfigured host takes the first
    // seed volume it finds, and it is to be its own, naming its
    // management NIC, never a guest's.
    let host_seed = run.file("host-seed.img");
    let host_toml = format!(
        "schema_version = 1\n\n[host]\nname = \"vs-net\"\n\n[network]\ndevice = \"{MANAGEMENT_MAC}\"\n"
    );
    seed_volume(&host_seed, release::SEED_LABEL, &host_toml)?;
    let mut units: Vec<PathBuf> = vec![host_seed];
    for i in 0..rig.guests {
        let disk = run.file(&format!("guest{i}.img"));
        sparse_copy(image, &disk)?;
        units.push(disk);
        let seed = run.file(&format!("seed{i}.img"));
        let text = rig
            .seeds
            .get(i)
            .cloned()
            .flatten()
            .unwrap_or_else(|| "schema_version = 1\n".into());
        seed_volume(&seed, release::SEED_LABEL, &text)?;
        units.push(seed);
    }
    units.extend(rig.disks.iter().cloned());
    let ports = Station::ports(rig.nics.len()).map_err(|e| e.to_string())?;
    let station = Station::start(&ports, rig.dhcp).map_err(|e| e.to_string())?;
    let mut l = base;
    let first = l.extra.len();
    for (i, path) in units.iter().enumerate() {
        l.extra.push(Drive {
            path: path.clone(),
            serial: format!("QSVSUNIT{i}"),
            place: l.pinned.then(|| u8::try_from(first + i).unwrap_or(0)),
        });
    }
    if l.nic.is_none() {
        l.nic = Some(MANAGEMENT_MAC.into());
    }
    if l.monitor.is_none() {
        l.monitor = Some(run.socket("monitor.sock"));
    }
    let n = ports.len();
    l.uplinks = ports
        .iter()
        .zip(&rig.nics)
        .enumerate()
        .map(|(i, (p, mtu))| UplinkNic {
            mac: uplink_mac(i),
            far: Far::Station(*p),
            mtu: *mtu,
        })
        .chain(rig.user.iter().enumerate().map(|(j, fwd)| UplinkNic {
            mac: uplink_mac(n + j),
            far: Far::User(fwd.clone()),
            mtu: None,
        }))
        .collect();
    Ok((l, station))
}

/// Boot a host of the test image for `rig` to its partition manager
/// answering.
pub fn boot<'a>(run: &Run<'a>, rig: &Rig) -> Result<Net<'a>, String> {
    fresh_host(run)?;
    let (l, station) = rig_launch(run, rig, launch(run, &[])?)?;
    let guest = host(run, &l)?;
    Ok(Net { guest, station })
}

/// The disk of guest `i` as the host has it.
pub fn guest_disk(i: usize) -> String {
    let letter = char::from(b'c' + u8::try_from(2 * i).unwrap_or(0));
    format!("/dev/vd{letter}")
}

/// Its seed volume.
pub fn guest_seed(i: usize) -> String {
    let letter = char::from(b'c' + u8::try_from(2 * i + 1).unwrap_or(0));
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

/// The switch service behind a channel.
pub struct Vswitchd<'a> {
    channel: &'a mut crate::qemu::channel::Channel,
}

impl vswitchd::proto::Calls for Vswitchd<'_> {
    fn call(&mut self, req: &vswitchd::proto::Request) -> Result<Vec<u8>, ipc::WireError> {
        let line = format!("{} {}", command::VSWITCHD, hex::encode(ipc::encode(req)));
        let lines = self
            .channel
            .ask_within(&line, Duration::from_secs(60))
            .map_err(std::io::Error::other)?;
        let answer = lines
            .first()
            .and_then(|l| l.strip_prefix(qsvshook::proto::word::HEX))
            .ok_or_else(|| std::io::Error::other(format!("`{}` answered {lines:?}", req.name())))?;
        Ok(hex::decode(answer.trim()).map_err(std::io::Error::other)?)
    }
}

/// The switch service of the host `guest` is.
pub fn vswitchd(guest: &mut Session) -> Result<Vswitchd<'_>, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    Ok(Vswitchd {
        channel: guest.channel(deadline)?,
    })
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
    guest.seen(&relayed(name, &posted(&refcode::BASE_READY, "")), deadline)
}

/// The tap of slot `slot` of partition `id`.
pub fn tap(id: u32, slot: u32) -> String {
    vswitch::switch::tap_name(id, slot)
}

/// The uplink's place on the PCI bus, for the record.
pub fn uplink_place(i: usize) -> u8 {
    FIRST_UPLINK_PLACE + u8::try_from(i).unwrap_or(0)
}
