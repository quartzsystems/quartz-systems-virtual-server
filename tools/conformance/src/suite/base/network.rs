// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 2, 3 and 10: DHCP on QEMU's user network, a static address
//! on a VLAN seeded on the second IPL (and a reload back to DHCP), and
//! that nothing listens.

use std::time::{Duration, Instant};

use netlink::procnet::{self, ProcSock};
use qsvshook::proto::command;
use release::{fact, screen};

use super::{
    DHCP_ADDRESS, DHCP_DNS, DHCP_GATEWAY, MAC, Run, STATE_DIR, booted, channel, fact_of, posted,
    screen_with, seed_toml,
};
use crate::qemu::image::{seed_volume, sparse_copy};
use crate::qemu::{Drive, Launch};

/// `TCP_LISTEN`.
const LISTEN: u8 = 10;
/// The DHCP client's port.
const DHCP_CLIENT: u16 = 68;

/// The management line of an address on an interface.
fn line(address: &str, interface: &str, gateway: &str) -> String {
    format!("{address} on {interface}, gateway {gateway}")
}

fn with_nic(run: &Run<'_>) -> Launch {
    Launch {
        nic: Some(MAC.to_string()),
        ..run.launch()
    }
}

/// Suite 2: DHCP. The expected address and gateway on the status
/// screen.
pub fn dhcp(run: &Run<'_>) -> Result<String, String> {
    let mut guest = booted(run, &with_nic(run))?;
    let deadline = run.deadline();
    let wanted = line(DHCP_ADDRESS, "eth0", DHCP_GATEWAY);
    guest.seen(&posted(&refcode::INTERFACE_UP, "eth0"), deadline)?;
    guest.seen(&posted(&refcode::ADDRESS_ACQUIRED, &wanted), deadline)?;
    screen_with(
        &mut guest,
        &[screen::row(screen::MANAGEMENT, &wanted)],
        deadline,
    )?;
    let channel = channel(&mut guest)?;
    let status = channel.status()?;
    if fact_of(&status, fact::MANAGEMENT)? != wanted {
        return Err(format!(
            "the status's management line is `{}`",
            fact_of(&status, fact::MANAGEMENT)?
        ));
    }
    let resolver = channel.text("/etc/resolv.conf")?;
    if resolver.trim() != format!("nameserver {DHCP_DNS}") {
        return Err(format!("the resolver's file is {resolver:?}"));
    }
    Ok(format!("{wanted}; name server {DHCP_DNS}"))
}

/// Suite 3: a seeded static configuration with a VLAN ID. The host
/// has been through one IPL without the seed volume and is not configured by
/// that; on the IPL with the volume the address is applied and the
/// VLAN interface exists. Then `network.toml` is changed to DHCP and
/// the service told to reload: it follows without a restart.
pub fn static_vlan(run: &Run<'_>) -> Result<String, String> {
    let disk = run.file("disk.img");
    sparse_copy(run.disk, &disk)?;
    let unseeded = Launch {
        disk: Some(disk),
        snapshot: false,
        ..with_nic(run)
    };
    {
        let mut first = booted(run, &unseeded)?;
        if !first.said(&posted(&refcode::SEED_CHECKED, "no seed volume")) {
            return Err("the first IPL did not look for a seed volume".into());
        }
        let files = channel(&mut first)?.ls(STATE_DIR)?;
        if !files.iter().any(|f| f == "identity.toml") || files.iter().any(|f| f == "host.toml") {
            return Err(format!("the state files of an unseeded host are {files:?}"));
        }
    }
    let seed = run.file("seed.img");
    let toml = seed_toml(
        "name = \"vs-seeded\"\n",
        "mode = \"static\"\naddress = \"192.0.2.10/24\"\ngateway = \"192.0.2.1\"\nvlan = 100\n\
         dns = [\"192.0.2.53\"]\n",
    );
    seed_volume(&seed, release::SEED_LABEL, &toml)?;
    let launch = Launch {
        extra: vec![Drive {
            path: seed,
            serial: "QSVSSEED0".into(),
        }],
        ..unseeded
    };
    let mut guest = run.boot(&launch)?;
    let deadline = run.deadline();
    guest.seen(&posted(&refcode::SEED_APPLIED, ""), deadline)?;
    guest.seen(&posted(&refcode::STATE_LOADED, "host vs-seeded"), deadline)?;
    let wanted = line("192.0.2.10/24", "eth0.100", "192.0.2.1");
    guest.seen(&posted(&refcode::ADDRESS_ACQUIRED, &wanted), deadline)?;
    screen_with(
        &mut guest,
        &[
            screen::row(screen::HOST, "vs-seeded"),
            screen::row(screen::MANAGEMENT, &wanted),
        ],
        deadline,
    )?;
    let channel = guest.channel(deadline)?;
    let links = channel.ls("/sys/class/net")?;
    if !links.iter().any(|l| l == "eth0.100") {
        return Err(format!("no eth0.100 among the links {links:?}"));
    }
    let netd = |status: &supervise::control::HostStatus| status.service("netd").map(|s| s.pid);
    let before = netd(&channel.status()?);

    // The reload: back to DHCP on the device itself.
    let dhcp = hex::encode("schema_version = 1\n");
    channel.ask(&format!("{} {STATE_DIR}/network.toml {dhcp}", command::PUT))?;
    channel.ask(command::NET_RELOAD)?;
    let after = line(DHCP_ADDRESS, "eth0", DHCP_GATEWAY);
    let status = channel.until("the reload to DHCP", deadline, |s| {
        s.fact(fact::MANAGEMENT) == Some(after.as_str())
    })?;
    if netd(&status) != before {
        return Err("the network service was restarted for the reload".into());
    }
    let links = channel.ls("/sys/class/net")?;
    if links.iter().any(|l| l == "eth0.100") {
        return Err("eth0.100 is still there after the reload".into());
    }
    Ok(format!(
        "seeded on the second IPL: {wanted}, eth0.100 exists; reloaded: {after}, eth0.100 gone, the same process"
    ))
}

/// Whether a socket is bound to an address that is not loopback.
fn off_loopback(s: &ProcSock) -> bool {
    !s.local.ip().is_loopback()
}

/// Suite 10: no listening sockets. No TCP socket listens, and no TCP
/// or UDP socket is bound off loopback but the DHCP client's and a
/// time client's own (a UDP socket connected to its server).
pub fn sockets(run: &Run<'_>) -> Result<String, String> {
    let mut guest = booted(run, &with_nic(run))?;
    let deadline = run.deadline();
    let wanted = line(DHCP_ADDRESS, "eth0", DHCP_GATEWAY);
    guest.seen(&posted(&refcode::ADDRESS_ACQUIRED, &wanted), deadline)?;
    let channel = channel(&mut guest)?;
    // The DHCP client holds its socket while it has a lease.
    let give_up = Instant::now() + Duration::from_secs(5);
    loop {
        let mut all = Vec::new();
        for (file, v6) in [
            ("tcp", false),
            ("tcp6", true),
            ("udp", false),
            ("udp6", true),
        ] {
            let text = channel.text(&format!("/proc/net/{file}"))?;
            let tcp = file.starts_with("tcp");
            all.extend(procnet::parse(&text, v6).into_iter().map(|s| (tcp, s)));
        }
        if let Some((_, s)) = all.iter().find(|(tcp, s)| *tcp && s.state == LISTEN) {
            return Err(format!("a TCP socket listens on {}", s.local));
        }
        let strangers: Vec<String> = all
            .iter()
            .filter(|(_, s)| off_loopback(s))
            .filter(|(tcp, s)| {
                let dhcp = !tcp && s.local.port() == DHCP_CLIENT;
                let client = !tcp && s.remote.port() != 0;
                !(dhcp || client)
            })
            .map(|(tcp, s)| format!("{} {}", if *tcp { "TCP" } else { "UDP" }, s.local))
            .collect();
        if !strangers.is_empty() {
            return Err(format!("sockets bound off loopback: {strangers:?}"));
        }
        let dhcp = all
            .iter()
            .any(|(tcp, s)| !tcp && s.local.port() == DHCP_CLIENT);
        if dhcp {
            let (tcp, udp) = (
                all.iter().filter(|(tcp, _)| *tcp).count(),
                all.iter().filter(|(tcp, _)| !tcp).count(),
            );
            return Ok(format!(
                "{tcp} TCP sockets, none listening; {udp} UDP socket(s): the DHCP client's on port {DHCP_CLIENT}"
            ));
        }
        if Instant::now() >= give_up {
            return Err("the DHCP client's socket is not there, though it has a lease".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
