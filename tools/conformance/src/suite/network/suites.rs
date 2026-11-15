// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The suites of Phase 6, in the order of the handoff. Those that go
//! through Service Tools are in [`super::tools`], QS/400's in
//! [`super::qs400`].

use std::time::{Duration, Instant};

use partd::Calls as _;
use vswitch::{Bond, Uplink};

use super::{
    Net, Rig, activate, adapter, boot, counter, guest_address, guest_ready, guest_spec, links,
    mac_of, ports, restart_service, set_switches, static_seed, switch, tap, uplink_location,
};
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::manager;
use crate::suite::partition::signal_to;

pub use super::tools::{create, live, management};

/// An uplink of the interfaces at `at`.
pub fn trunk(at: &[String], bond: Option<Bond>, native: Option<u16>) -> Uplink {
    Uplink {
        interfaces: at.to_vec(),
        bond,
        native_vlan: native,
    }
}

/// Wait until `f` says so, by `within`.
pub fn wait_until(
    what: &str,
    within: Duration,
    mut f: impl FnMut() -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + within;
    loop {
        if f()? {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("{what}: not within {within:?}"));
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Three cheap guests with static addresses on an internal switch, each
/// asking a time server: A and B ask each other on VLAN 10, C asks A
/// from VLAN 20. Suites 2 and 14.
struct Internal<'a> {
    net: Net<'a>,
    ids: [u32; 3],
}

fn three<'a>(run: &Run<'a>) -> Result<Internal<'a>, String> {
    let mut rig = Rig::new(0, 3);
    rig.seeds = vec![
        Some(static_seed("neta", "10.1.0.1/24", "10.1.0.2")),
        Some(static_seed("netb", "10.1.0.2/24", "10.1.0.1")),
        Some(static_seed("netc", "10.1.0.3/24", "10.1.0.1")),
    ];
    let mut net = boot(run, &rig)?;
    let g = &mut net.guest;
    set_switches(g, &[switch("INTERNAL", 1, &[10, 20], 1500, None)])?;
    let mut ids = [0; 3];
    for (i, (name, vlan)) in [("NETA", 10), ("NETB", 10), ("NETC", 20)].iter().enumerate() {
        ids[i] = activate(g, name, &guest_spec(i, vec![adapter(5, "INTERNAL", *vlan, &[])]))?;
    }
    let deadline = run.deadline();
    for name in ["NETA", "NETB", "NETC"] {
        guest_ready(g, name, deadline)?;
    }
    // Their time clients ask, and ask again.
    std::thread::sleep(Duration::from_secs(20));
    Ok(Internal { net, ids })
}

/// What each port's partition sent and was sent.
fn traffic(net: &mut Net<'_>, ids: &[u32]) -> Result<Vec<(u64, u64)>, String> {
    ids.iter()
        .map(|id| {
            let t = tap(*id, 5);
            Ok((
                counter(&mut net.guest, &t, "rx_packets")?,
                counter(&mut net.guest, &t, "tx_packets")?,
            ))
        })
        .collect()
}

/// Suite 2: two partitions on one VLAN of an internal switch reach each
/// other; a third on another VLAN reaches neither and is reached by
/// neither.
pub fn internal(run: &Run<'_>) -> Result<String, String> {
    let mut t = three(run)?;
    let c = traffic(&mut t.net, &t.ids)?;
    let ((a_sent, a_got), (b_sent, b_got), (c_sent, c_got)) = (c[0], c[1], c[2]);
    // On VLAN 10 there are A and B alone: what each was sent, the other
    // sent. C's frames reach nobody, and nothing reaches C.
    if a_got == 0 || b_got == 0 {
        return Err(format!("A and B do not reach each other: {c:?}"));
    }
    if a_got > b_sent || b_got > a_sent {
        return Err(format!("A or B was sent more than the other sent: {c:?}"));
    }
    if c_sent == 0 || c_got != 0 {
        return Err(format!("C sent {c_sent} and was sent {c_got}: {c:?}"));
    }
    Ok(format!(
        "A sent {a_sent} frames and was sent {a_got}, B sent {b_sent} and was sent {b_got}: each \
         reaches the other and nothing else does; C on VLAN 20 sent {c_sent}, was sent 0"
    ))
}

/// Suite 3: a partition gets an address by DHCP from the harness's
/// network through the uplink, its frames leaving tagged with its VLAN;
/// with that VLAN native they leave untagged.
pub fn uplink(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, &Rig::new(1, 1))?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?];
    set_switches(g, &[switch("PRODNET", 1, &[10, 20], 1500, Some(trunk(&at, None, None)))])?;
    let id = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let mac = mac_of(g, id, 5)?;
    let address = guest_address(g, "NETA", run.deadline())?;
    if net.station.lease(10, mac) != Some(address) {
        return Err(format!(
            "the guest says {address}; the station gave {:?}",
            net.station.leases()
        ));
    }
    let from_guest: Vec<_> = net
        .station
        .seen()
        .into_iter()
        .filter(|s| s.src == mac)
        .collect();
    if from_guest.is_empty() || from_guest.iter().any(|s| s.vlan != Some(10)) {
        return Err(format!("the guest's frames on the uplink: {from_guest:?}"));
    }
    let (got, took) = net
        .station
        .ping(0, 10, address, 10, 56, Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    if got != 10 {
        return Err(format!("{got} of 10 echo replies on VLAN 10"));
    }
    set_switches(
        &mut net.guest,
        &[switch("PRODNET", 1, &[10, 20], 1500, Some(trunk(&at, None, Some(10))))],
    )?;
    net.station.clear();
    let (native, _) = net
        .station
        .ping(0, 0, address, 10, 56, Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let tagged = net
        .station
        .seen()
        .into_iter()
        .filter(|s| s.src == mac && s.vlan.is_some())
        .count();
    if native != 10 || tagged != 0 {
        return Err(format!(
            "native VLAN 10: {native} of 10 replies untagged, {tagged} tagged frames"
        ));
    }
    Ok(format!(
        "{} got {address} by DHCP through uplink {}, {} frame(s) of it tagged VLAN 10; 10 of 10 \
         echoes in {took:?}; with VLAN 10 native, 10 of 10 untagged and none tagged",
        vswitch::mac::text(&mac),
        at[0],
        from_guest.len()
    ))
}

/// The seed of a guest whose network service asks DHCP on VLAN `vlan`
/// of its link: an 802.1Q interface of its own.
fn vlan_seed(name: &str, vlan: u16) -> String {
    format!("schema_version = 1\n\n[host]\nname = \"{name}\"\n\n[network]\nvlan = {vlan}\n")
}

/// Suite 4: a tagged adapter: the guest sends and receives on a listed
/// VLAN with its tags; a frame on a VLAN not listed is dropped either
/// way.
pub fn tagged(run: &Run<'_>) -> Result<String, String> {
    let mut rig = Rig::new(1, 2);
    rig.seeds = vec![Some(vlan_seed("neta", 20)), Some(vlan_seed("netb", 30))];
    let mut net = boot(run, &rig)?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?];
    set_switches(
        g,
        &[switch("PRODNET", 1, &[10, 20, 30], 1500, Some(trunk(&at, None, None)))],
    )?;
    // Both on VLAN 10, VLAN 20 tagged; B asks on VLAN 30, not listed.
    let a = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[20])]))?;
    let b = activate(g, "NETB", &guest_spec(1, vec![adapter(5, "PRODNET", 10, &[20])]))?;
    let (ma, mb) = (mac_of(g, a, 5)?, mac_of(g, b, 5)?);
    let address = guest_address(g, "NETA", run.deadline())?;
    if net.station.lease(20, ma) != Some(address) {
        return Err(format!(
            "A says {address}; the station gave {:?}",
            net.station.leases()
        ));
    }
    let (got, _) = net
        .station
        .ping(0, 20, address, 10, 56, Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    if got != 10 {
        return Err(format!("{got} of 10 echo replies on tagged VLAN 20"));
    }
    guest_ready(&mut net.guest, "NETB", run.deadline())?;
    std::thread::sleep(Duration::from_secs(10));
    let from_b: Vec<_> = net
        .station
        .seen()
        .into_iter()
        .filter(|s| s.src == mb)
        .collect();
    if from_b.iter().any(|s| s.vlan == Some(30)) {
        return Err(format!("B's frames on VLAN 30 left the port: {from_b:?}"));
    }
    // From outside on VLAN 30: B is not sent it.
    let before = counter(&mut net.guest, &tap(b, 5), "tx_packets")?;
    for _ in 0..20 {
        let f = crate::qemu::wire::frame([0xFF; 6], crate::qemu::wire::MAC, Some(30), 0x88B5, &[1]);
        net.station.send(0, &f).map_err(|e| e.to_string())?;
    }
    std::thread::sleep(Duration::from_secs(1));
    let after = counter(&mut net.guest, &tap(b, 5), "tx_packets")?;
    if after != before {
        return Err(format!("B was sent {} frames of VLAN 30", after - before));
    }
    Ok(format!(
        "A got {address} by DHCP on tagged VLAN 20 and answered 10 of 10 echoes on it; B's DHCP on \
         VLAN 30, not listed, never left its port ({} frames of B on the uplink, none tagged 30), \
         and 20 broadcasts on VLAN 30 from outside reached it 0 times",
        from_b.len()
    ))
}

/// Suite 5: what a partition may not do, here what a cheap guest can be
/// made to try: a tagged frame from an access adapter (the guest's DHCP
/// on a VLAN interface of its own) is dropped at the port, whether the
/// tag is another VLAN's or its own. A frame from another source address
/// and an address changed through the control queue are shown by
/// `net/nl`'s live test and the device's unit tests (`docs/PHASE6.md`).
pub fn may_not(run: &Run<'_>) -> Result<String, String> {
    let mut rig = Rig::new(1, 2);
    rig.seeds = vec![Some(vlan_seed("neta", 20)), Some(vlan_seed("netb", 10))];
    let mut net = boot(run, &rig)?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?];
    set_switches(
        g,
        &[switch("PRODNET", 1, &[10, 20], 1500, Some(trunk(&at, None, None)))],
    )?;
    let a = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let b = activate(g, "NETB", &guest_spec(1, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let (ma, mb) = (mac_of(g, a, 5)?, mac_of(g, b, 5)?);
    let deadline = run.deadline();
    guest_ready(g, "NETA", deadline)?;
    guest_ready(g, "NETB", deadline)?;
    std::thread::sleep(Duration::from_secs(15));
    let seen = net.station.seen();
    let ipv4_from = |mac| {
        seen.iter()
            .filter(|s| s.src == mac && s.ethertype == 0x0800)
            .count()
    };
    let (sa, sb) = (
        counter(&mut net.guest, &tap(a, 5), "rx_packets")?,
        counter(&mut net.guest, &tap(b, 5), "rx_packets")?,
    );
    if ipv4_from(ma) != 0 || ipv4_from(mb) != 0 || sa == 0 || sb == 0 {
        return Err(format!(
            "A sent {sa} and B {sb} frames; IPv4 on the uplink from A {} and from B {}",
            ipv4_from(ma),
            ipv4_from(mb)
        ));
    }
    Ok(format!(
        "access adapters on VLAN 10: A's DHCP tagged VLAN 20 and B's tagged with its own VLAN 10 \
         were dropped at their ports ({sa} and {sb} frames sent, no IPv4 of either on the uplink)"
    ))
}

/// Suite 6: the host is unreachable from a partition on every switch,
/// IPv4 and IPv6, and forwards nothing.
pub fn host_unreachable(run: &Run<'_>) -> Result<String, String> {
    let mut rig = Rig::new(1, 2);
    // A asks the host's management address for the time; B, on another
    // switch, asks A's: across switches.
    rig.seeds = vec![
        Some(static_seed("neta", "10.0.2.99/24", "10.0.2.15")),
        Some(static_seed("netb", "10.0.2.98/24", "10.0.2.99")),
    ];
    let mut net = boot(run, &rig)?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?];
    set_switches(
        g,
        &[
            switch("INTERNAL", 1, &[10], 1500, None),
            switch("PRODNET", 2, &[10], 1500, Some(trunk(&at, None, Some(10)))),
        ],
    )?;
    let a = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "INTERNAL", 10, &[])]))?;
    let b = activate(g, "NETB", &guest_spec(1, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let deadline = run.deadline();
    guest_ready(g, "NETA", deadline)?;
    guest_ready(g, "NETB", deadline)?;
    std::thread::sleep(Duration::from_secs(20));
    let mut found = Vec::new();
    let (ta, tb) = (tap(a, 5), tap(b, 5));
    let c = channel(&mut net.guest)?;
    for link in ["qvbr1", "qvbr2", ta.as_str(), tb.as_str()] {
        let off = c.text(&format!("/proc/sys/net/ipv6/conf/{link}/disable_ipv6"))?;
        if off.trim() != "1" {
            found.push(format!("{link} has IPv6"));
        }
    }
    let v6 = c.text("/proc/net/if_inet6")?;
    let routes = c.text("/proc/net/route")?;
    for link in ["qvbr1", "qvbr2", "qvt"] {
        if v6.lines().any(|l| l.contains(link)) {
            found.push(format!("an IPv6 address on {link}"));
        }
        if routes.lines().any(|l| l.starts_with(link)) {
            found.push(format!("an IPv4 route on {link}"));
        }
    }
    for key in ["ipv4/ip_forward", "ipv6/conf/all/forwarding"] {
        if c.text(&format!("/proc/sys/net/{key}"))?.trim() != "0" {
            found.push(format!("{key} is on"));
        }
    }
    let up1 = counter(&mut net.guest, "qvbr1", "rx_packets")?;
    let up2 = counter(&mut net.guest, "qvbr2", "rx_packets")?;
    if up1 != 0 || up2 != 0 {
        found.push(format!("the bridges passed {up1} and {up2} frames to the host"));
    }
    let ma = mac_of(&mut net.guest, a, 5)?;
    let seen_a = net.station.seen().iter().filter(|s| s.src == ma).count();
    let sent_a = counter(&mut net.guest, &ta, "rx_packets")?;
    if seen_a != 0 {
        found.push(format!("{seen_a} of A's frames left by another switch's uplink"));
    }
    if !found.is_empty() {
        return Err(found.join("; "));
    }
    Ok(format!(
        "A on an internal switch asked the host's management address and sent {sent_a} frames; B \
         on another switch asked A: no address of the host's and no route on either bridge or tap, \
         IPv6 off on each, forwarding off, 0 frames passed up to the host by either bridge, none \
         of A's left by the other switch"
    ))
}

/// Suite 8: an uplink's loss and return: the code and the event name the
/// interface by location code, outside traffic comes back; with a
/// two-member bond one member down loses no connectivity.
pub fn uplink_loss(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, &Rig::new(2, 1))?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?, uplink_location(g, 1)?];
    set_switches(
        g,
        &[switch(
            "PRODNET",
            1,
            &[10],
            1500,
            Some(trunk(&at, Some(Bond::ActiveBackup), Some(10))),
        )],
    )?;
    let a = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let address = guest_address(g, "NETA", run.deadline())?;
    let ping = |net: &Net<'_>| -> Result<usize, String> {
        let mut best = 0;
        for nic in 0..2 {
            let (got, _) = net
                .station
                .ping(nic, 0, address, 5, 56, Duration::from_secs(3))
                .map_err(|e| e.to_string())?;
            best = best.max(got);
        }
        Ok(best)
    };
    if ping(&net)? != 5 {
        return Err("no echo through the bond before anything".into());
    }
    let deadline = run.deadline();
    net.guest.monitor("set_link up0 off")?;
    net.station.cut(0, true);
    net.guest
        .expect(&posted(&refcode::UPLINK_MEMBER_DOWN, ""), deadline)?;
    let named = net.guest.rest_of_line(deadline)?;
    std::thread::sleep(Duration::from_secs(2));
    let one_down = ping(&net)?;
    net.guest.monitor("set_link up1 off")?;
    net.station.cut(1, true);
    net.guest.expect(&posted(&refcode::UPLINK_LOST, ""), deadline)?;
    let lost = net.guest.rest_of_line(deadline)?;
    let sent = counter(&mut net.guest, &tap(a, 5), "rx_packets")?;
    net.guest.monitor("set_link up0 on")?;
    net.station.cut(0, false);
    net.guest
        .expect(&posted(&refcode::UPLINK_RETURNED, ""), deadline)?;
    let back = net.guest.rest_of_line(deadline)?;
    std::thread::sleep(Duration::from_secs(3));
    let again = ping(&net)?;
    if !named.contains(&at[0]) || !lost.contains(&at[1]) || !back.contains(&at[0]) {
        return Err(format!("the codes named {named:?}, {lost:?}, {back:?}"));
    }
    if one_down != 5 || again != 5 || sent == 0 {
        return Err(format!(
            "echoes with one member down {one_down} of 5, after the return {again} of 5"
        ));
    }
    Ok(format!(
        "one member down: A2008025 named {}, 5 of 5 echoes through the other; both down: \
         A2008020 named {}; back: C2008020 named {}, 5 of 5 echoes again",
        at[0], at[1], at[0]
    ))
}

/// Suite 9.
pub fn qs400(run: &Run<'_>) -> Result<String, String> {
    super::qs400::suite(run)
}

/// Suite 11: the switch service killed under traffic: traffic goes on,
/// it finds its switches and ports, a partition activated afterwards
/// gets its port.
pub fn service_kill(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, &Rig::new(1, 2))?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?];
    set_switches(g, &[switch("PRODNET", 1, &[10], 1500, Some(trunk(&at, None, Some(10))))])?;
    let a = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let address = guest_address(g, "NETA", run.deadline())?;
    let before = ports(&mut net.guest)?;
    let mut lost = 0;
    let mut kills = 0;
    for round in 0..6 {
        if round % 2 == 1 {
            restart_service(&mut net.guest)?;
            kills += 1;
        }
        let (got, _) = net
            .station
            .ping(0, 0, address, 20, 56, Duration::from_secs(3))
            .map_err(|e| e.to_string())?;
        lost += 20 - got;
    }
    let after = ports(&mut net.guest)?;
    if after != before || lost > 2 {
        return Err(format!(
            "{lost} of 120 echoes lost; ports before {before:?}, after {after:?}"
        ));
    }
    let b = activate(
        &mut net.guest,
        "NETB",
        &guest_spec(1, vec![adapter(5, "PRODNET", 10, &[])]),
    )?;
    guest_address(&mut net.guest, "NETB", run.deadline())?;
    let now = ports(&mut net.guest)?;
    if !now.iter().any(|p| p.partition == b) || !now.iter().any(|p| p.partition == a) {
        return Err(format!("the ports after B's activation: {now:?}"));
    }
    Ok(format!(
        "the switch service killed {kills} times under 120 echoes: {lost} lost, its port found \
         again each time; B activated after got its port and an address"
    ))
}

/// Suite 12: a killed `qsvmm` and a killed `partd`: no tap or port is
/// left once things settle, and the running partitions' ports are
/// untouched.
pub fn vmm_kill(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, &Rig::new(0, 2))?;
    let g = &mut net.guest;
    set_switches(g, &[switch("INTERNAL", 1, &[10], 1500, None)])?;
    let a = activate(g, "NETA", &guest_spec(0, vec![adapter(5, "INTERNAL", 10, &[])]))?;
    let b = activate(g, "NETB", &guest_spec(1, vec![adapter(5, "INTERNAL", 10, &[])]))?;
    let deadline = run.deadline();
    guest_ready(g, "NETA", deadline)?;
    guest_ready(g, "NETB", deadline)?;
    let (ta, tb) = (tap(a, 5), tap(b, 5));
    signal_to(g, a, "KILL")?;
    wait_until("A's tap and port gone", Duration::from_secs(20), || {
        let l = links(g)?;
        Ok(!l.contains(&ta) && !ports(g)?.iter().any(|p| p.partition == a))
    })?;
    if !links(g)?.contains(&tb) {
        return Err("B's tap went with A's VMM".into());
    }
    let b_sent = counter(g, &tb, "rx_packets")?;
    channel(g)?.ask(&format!(
        "{} {}",
        qsvshook::proto::command::KILL,
        partd::proto::NAME
    ))?;
    std::thread::sleep(Duration::from_secs(1));
    wait_until("the partition manager again", Duration::from_secs(60), || {
        Ok(manager::partd(g)?.list().is_ok())
    })?;
    std::thread::sleep(Duration::from_secs(3));
    let now = ports(g)?;
    let taps: Vec<String> = links(g)?
        .into_iter()
        .filter(|l| l.starts_with("qvt"))
        .collect();
    if now.len() != 1 || now[0].partition != b || taps != [tb.clone()] {
        return Err(format!(
            "after the partition manager's restart: ports {now:?}, taps {taps:?}"
        ));
    }
    if counter(g, &tb, "rx_packets")? < b_sent {
        return Err("B's port was made again".into());
    }
    Ok(format!(
        "A's qsvmm killed: its tap {ta} and port gone within seconds, B's untouched; the partition \
         manager killed and started again: B's port {tb} the only one left, its counters going on"
    ))
}

/// Suite 13: an adapter that is not required on a switch that is gone
/// comes up without link; a required one is refused by name.
pub fn not_required(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, &Rig::new(0, 1))?;
    let g = &mut net.guest;
    set_switches(
        g,
        &[
            switch("GONE", 1, &[10], 1500, None),
            switch("KEPT", 2, &[10], 1500, None),
        ],
    )?;
    let id = manager::partition(g, "NETA")?;
    let mut optional = guest_spec(0, vec![adapter(5, "GONE", 10, &[])]);
    optional.ethernet[0].required = false;
    optional.name = "OPTIONAL".into();
    let mut required = guest_spec(0, vec![adapter(5, "GONE", 10, &[])]);
    required.name = "REQUIRED".into();
    manager::put(g, id, &optional)?;
    manager::put(g, id, &required)?;
    // The switch is gone from the host's state.
    set_switches(g, &[switch("KEPT", 2, &[10], 1500, None)])?;
    wait_until("GONE's bridge deleted", Duration::from_secs(10), || {
        Ok(!links(g)?.contains(&"qvbr1".to_string()))
    })?;
    let refused = manager::partd(g)?
        .activate(id, "REQUIRED".into(), None, None)
        .map_err(|e| e.to_string())?
        .err()
        .ok_or("the required adapter's activation was not refused")?;
    let first = refused.first().map(ToString::to_string).unwrap_or_default();
    if !refused[0].is(&qvsmsg::PORT_NOT_GIVEN) || !first.contains("slot 5") {
        return Err(format!("refused with {}", manager::refused(&refused)));
    }
    manager::said(manager::partd(g)?.activate(id, "OPTIONAL".into(), None, None))?;
    guest_ready(g, "NETA", run.deadline())?;
    let status = manager::partd(g)?
        .list()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("no such partition")?;
    let link = status.ethernet.first().map(|e| e.link);
    if status.state != partd::State::Running
        || link != Some(false)
        || links(g)?.contains(&tap(id, 5))
    {
        return Err(format!(
            "the optional activation: {:?}, link {link:?}",
            status.state
        ));
    }
    Ok(format!(
        "switch GONE taken away under a profile that names it: the required adapter refused with \
         `{first}`; not required, the partition runs with the adapter's link down and no port"
    ))
}

/// Suite 14: the counters and the forwarding table say what the traffic
/// of suite 2 implies.
pub fn counters(run: &Run<'_>) -> Result<String, String> {
    let mut t = three(run)?;
    let c = traffic(&mut t.net, &t.ids)?;
    if c[0].1 > c[1].0 || c[1].1 > c[0].0 || c[2].1 != 0 || c[0].1 == 0 {
        return Err(format!("the counters {c:?}"));
    }
    let g = &mut t.net.guest;
    let raw = channel(g)?.cat("/sys/class/net/qvbr1/brforward")?;
    let mut table = Vec::new();
    for e in raw.chunks_exact(16) {
        // struct __fdb_entry: mac, port_no, is_local, ageing, port_hi.
        let mac = <[u8; 6]>::try_from(&e[0..6]).unwrap_or_default();
        let port = u16::from(e[6]) | (u16::from(e[12]) << 8);
        table.push((mac, port, e[7] != 0));
    }
    let mut found = Vec::new();
    for id in t.ids {
        let mac = mac_of(g, id, 5)?;
        let port_no = channel(g)?.text(&format!(
            "/sys/class/net/{}/brport/port_no",
            tap(id, 5)
        ))?;
        let port_no =
            u16::from_str_radix(port_no.trim().trim_start_matches("0x"), 16).unwrap_or(0);
        if !table.iter().any(|(m, p, _)| *m == mac && *p == port_no) {
            return Err(format!(
                "{} is not in the table on port {port_no}: {table:?}",
                vswitch::mac::text(&mac)
            ));
        }
        found.push(format!("{} on port {port_no}", vswitch::mac::text(&mac)));
    }
    let more = table.len().saturating_sub(found.len());
    Ok(format!(
        "counters (sent, was sent) A {:?} B {:?} C {:?}: each of A and B was sent no more than the \
         other sent, C nothing; the table has {} ({more} more)",
        c[0],
        c[1],
        c[2],
        found.join(", ")
    ))
}

/// Suite 15: a throughput baseline through the uplink: echoes from the
/// station to a partition, as fast as they go. Between two partitions
/// the cheap guest makes no traffic to measure (`docs/PHASE6.md`).
pub fn throughput(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, &Rig::new(1, 1))?;
    let g = &mut net.guest;
    let at = vec![uplink_location(g, 0)?];
    set_switches(g, &[switch("PRODNET", 1, &[10], 1500, Some(trunk(&at, None, Some(10))))])?;
    activate(g, "NETA", &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]))?;
    let address = guest_address(g, "NETA", run.deadline())?;
    let mut rows = Vec::new();
    for (count, size) in [(2000u16, 1472usize), (2000, 56)] {
        let (got, took) = net
            .station
            .ping(0, 0, address, count, size, Duration::from_secs(20))
            .map_err(|e| e.to_string())?;
        let secs = took.as_secs_f64().max(1e-6);
        #[allow(clippy::cast_precision_loss)]
        let (replies, bits) = (got as f64, (got * (size + 42) * 8 * 2) as f64);
        rows.push(format!(
            "{size}-byte echoes: {got} of {count} in {secs:.2} s, {:.0} replies/s, {:.1} Mb/s both ways",
            replies / secs,
            bits / secs / 1e6
        ));
    }
    Ok(rows.join("; "))
}
