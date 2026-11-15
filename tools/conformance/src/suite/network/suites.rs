// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The suites of Phase 6, in the order of the handoff.

use std::time::Duration;

use vswitch::Uplink;

use super::{
    activate, adapter, boot, guest_address, guest_spec, mac_of, set_switches, switch,
    uplink_location,
};
use crate::suite::base::Run;

fn not_yet(name: &str) -> Result<String, String> {
    Err(format!("{name}: not written yet"))
}

/// Suite 1.
pub fn create(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-switch-create")
}

/// Suite 2.
pub fn internal(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-internal")
}

/// Suite 3: a partition gets an address by DHCP from the harness's
/// network through the uplink, its frames leaving tagged with its VLAN;
/// with that VLAN native they leave untagged.
pub fn uplink(run: &Run<'_>) -> Result<String, String> {
    let mut net = boot(run, 1, &[], 1, &[], true)?;
    let g = &mut net.guest;
    let at = uplink_location(g, 0)?;
    let trunk = |native| Uplink {
        interfaces: vec![at.clone()],
        bond: None,
        native_vlan: native,
    };
    set_switches(
        g,
        &[switch("PRODNET", 1, &[10, 20], 1500, Some(trunk(None)))],
    )?;
    let id = activate(
        g,
        "NETA",
        &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]),
    )?;
    let mac = mac_of(g, id, 5)?;
    let deadline = run.deadline();
    let address = guest_address(g, "NETA", deadline)?;
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
    // Native: the same VLAN untagged on the uplink.
    set_switches(
        g,
        &[switch("PRODNET", 1, &[10, 20], 1500, Some(trunk(Some(10))))],
    )?;
    net.station.clear();
    let (native, _) = net
        .station
        .ping(0, 0, address, 10, 56, Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    let tagged: Vec<_> = net
        .station
        .seen()
        .into_iter()
        .filter(|s| s.src == mac && s.vlan.is_some())
        .collect();
    if native != 10 || !tagged.is_empty() {
        return Err(format!(
            "native VLAN 10: {native} of 10 replies untagged, {} tagged frames",
            tagged.len()
        ));
    }
    Ok(format!(
        "{} got {address} by DHCP through uplink {at}, {} frame(s) of it tagged VLAN 10; 10 of 10 \
         echoes in {took:?}; with VLAN 10 native, 10 of 10 untagged and none tagged",
        vswitch::mac::text(&mac),
        from_guest.len()
    ))
}

/// Suite 4.
pub fn tagged(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-tagged")
}

/// Suite 5.
pub fn may_not(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-may-not")
}

/// Suite 6.
pub fn host_unreachable(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-host-unreachable")
}

/// Suite 7.
pub fn management(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-management")
}

/// Suite 8.
pub fn uplink_loss(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-uplink-loss")
}

/// Suite 9.
pub fn qs400(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-qs400")
}

/// Suite 10.
pub fn live(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-live")
}

/// Suite 11.
pub fn service_kill(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-service-kill")
}

/// Suite 12.
pub fn vmm_kill(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-vmm-kill")
}

/// Suite 13.
pub fn not_required(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-not-required")
}

/// Suite 14.
pub fn counters(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-counters")
}

/// Suite 15.
pub fn throughput(_run: &Run<'_>) -> Result<String, String> {
    not_yet("net-throughput")
}
