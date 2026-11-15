// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The suites that go through Service Tools option 3, on an installed
//! host with its service tools users: a switch made from the console
//! (suite 1), the management interface refused as an uplink (suite 7),
//! and live changes (suite 10). Each screen is held against its golden
//! file.

use std::time::{Duration, Instant};

use partd::Calls as _;

use super::suites::wait_until;
use super::{
    Net, Rig, activate, adapter, guest_address, guest_spec, links, rig_launch, uplink_mac,
};
use crate::qemu::term::key;
use crate::suite::base::{Guest, Run, channel};
use crate::suite::install::{
    Kind, PASSWORD, USER, boot, fill, golden, installed, machine, option, power_off, sign_on,
    titled,
};
use crate::suite::partition::{HOST_CPUS, HOST_MEMORY_MB, manager};

const SWITCHES: &str = "Work with Virtual Switches";
const NETWORK: &str = "Network";
use super::MANAGEMENT_MAC;

/// An installed host for `rig`, signed on at the main menu.
fn installed_host<'a>(
    run: &Run<'a>,
    rig: &Rig,
    management: bool,
) -> Result<(Net<'a>, crate::qemu::Launch), String> {
    crate::suite::partition::fresh_host(run)?;
    let disks = installed(run, Kind::Single)?;
    let mut base = machine(run, &[Some(disks[0].as_path())]);
    base.keys = true;
    base.memory_mb = HOST_MEMORY_MB;
    base.cpus = HOST_CPUS;
    base.prealloc = true;
    if management {
        base.nic = Some(MANAGEMENT_MAC.into());
    }
    let (l, station) = rig_launch(run, rig, base)?;
    let mut guest = boot(run, &l)?;
    let deadline = Instant::now() + 3 * run.timeout;
    wait_until("the partition manager", Duration::from_secs(60), || {
        Ok(manager::partd(&mut guest)?.list().is_ok())
    })?;
    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    Ok((Net { guest, station }, l))
}

/// From the main menu to the switches.
fn to_switches(g: &mut Guest<'_>, deadline: Instant) -> Result<String, String> {
    option(g, 3, NETWORK, deadline)?;
    option(g, 3, SWITCHES, deadline)
}

/// Make switch `name` from the list: F6, the form, Enter.
fn make(
    g: &mut Guest<'_>,
    name: &str,
    vlans: &str,
    mtu: &str,
    deadline: Instant,
) -> Result<String, String> {
    g.type_keys(key::F6)?;
    let form = titled(g, "Create Virtual Switch", deadline)?;
    g.type_keys(&format!(
        "{}{}",
        fill(&[Some(name), Some(vlans), Some(mtu)], &[10, 40, 4]),
        key::ENTER
    ))?;
    let said = format!("Switch {name} is as given.");
    g.screen("the switch made", deadline, |grid| grid.contains(&said))?;
    Ok(form)
}

/// Type option `n` by row `row` of a list of `rows`, then Enter.
fn by_row(g: &mut Guest<'_>, row: usize, rows: usize, n: &str) -> Result<(), String> {
    let mut values: Vec<Option<&str>> = vec![None; rows];
    values[row] = Some(n);
    g.type_keys(&format!("{}{}", fill(&values, &vec![1; rows]), key::ENTER))
}

/// The row of the interface `mac` in the host's lists: the inventory's
/// order.
fn row_of(g: &mut Guest<'_>, mac: &str) -> Result<(usize, usize), String> {
    let text = channel(g)?.text("/data/state/inventory.toml")?;
    let inventory: state::Inventory =
        state::store::parse(&text).map_err(|e| format!("inventory.toml: {e}"))?;
    let all = inventory.interfaces;
    let row = all
        .iter()
        .position(|i| i.mac.eq_ignore_ascii_case(mac))
        .ok_or_else(|| format!("no interface {mac}"))?;
    Ok((row, all.len()))
}

/// Set switch `row`'s uplink to the interface `mac`, with `mode` and
/// `native`; what the screen then says.
fn set_uplink(
    g: &mut Guest<'_>,
    switch_row: usize,
    switches: usize,
    mac: &str,
    native: &str,
    deadline: Instant,
) -> Result<String, String> {
    by_row(g, switch_row, switches, "7")?;
    titled(g, "Set Uplink", deadline)?;
    let (row, rows) = row_of(g, mac)?;
    by_row(g, row, rows, "1")?;
    g.screen("the uplink's form", deadline, |grid| {
        grid.contains("Bond mode")
    })?;
    g.type_keys(&format!(
        "{}{}",
        fill(&[Some(""), Some(native)], &[13, 4]),
        key::ENTER
    ))?;
    std::thread::sleep(Duration::from_secs(2));
    g.drain();
    let shown = g.screen("the uplink's answer", deadline, |grid| {
        !grid.message().is_empty()
    })?;
    Ok(shown)
}

/// Suite 1: an internal switch made from Service Tools; it survives a
/// host restart and a power pull.
pub fn create(run: &Run<'_>) -> Result<String, String> {
    let (mut net, l) = installed_host(run, &Rig::new(1, 0), false)?;
    let g = &mut net.guest;
    let deadline = Instant::now() + 3 * run.timeout;
    let menu = option(g, 3, NETWORK, deadline)?;
    golden("tools-network-menu", &menu)?;
    let hardware = option(g, 2, "Display Network Hardware", deadline)?;
    golden("tools-network-hardware", &hardware)?;
    g.type_keys(key::F12)?;
    titled(g, NETWORK, deadline)?;
    let empty = option(g, 3, SWITCHES, deadline)?;
    golden("tools-switches-empty", &empty)?;
    let form = make(g, "PRODNET", "10 20", "1500", deadline)?;
    golden("tools-switch-create", &form)?;
    g.drain();
    let listed = titled(g, SWITCHES, deadline)?;
    golden("tools-switches", &listed)?;
    by_row(g, 0, 1, "5")?;
    let shown = titled(g, "Display Virtual Switch", deadline)?;
    golden("tools-switch-display", &shown)?;
    g.type_keys(key::ENTER)?;
    titled(g, SWITCHES, deadline)?;
    let kept = |g: &mut Guest<'_>| -> Result<(), String> {
        let state = channel(g)?.text("/data/state/vswitch.toml")?;
        if !state.contains("PRODNET") || !links(g)?.contains(&"qvbr1".to_string()) {
            return Err(format!("the switch is not there: {state}"));
        }
        Ok(())
    };
    kept(g)?;
    // A host restart, then a power pull.
    power_off(g)?;
    let mut again = boot(run, &l)?;
    kept(&mut again)?;
    again.kill();
    let mut after = boot(run, &l)?;
    kept(&mut after)?;
    power_off(&mut after)?;
    Ok(
        "Service Tools option 3, 3, F6 made PRODNET with VLANs 10 and 20; there after a host \
        restart and after a power pull; golden files tools-network-menu, \
        tools-network-hardware, tools-switches-empty, tools-switch-create, tools-switches, \
        tools-switch-display"
            .into(),
    )
}

/// Suite 7: the management interface is refused as an uplink, and the
/// management network is undisturbed by a switch and its uplink.
pub fn management(run: &Run<'_>) -> Result<String, String> {
    let (mut net, _) = installed_host(run, &Rig::new(1, 0), true)?;
    let g = &mut net.guest;
    let deadline = Instant::now() + 3 * run.timeout;
    let before = channel(g)?
        .status()?
        .fact(release::fact::MANAGEMENT)
        .unwrap_or("")
        .to_string();
    to_switches(g, deadline)?;
    make(g, "PRODNET", "10", "1500", deadline)?;
    let refused = set_uplink(g, 0, 1, MANAGEMENT_MAC, "", deadline)?;
    if !refused.contains("QVS4102") {
        return Err(format!(
            "the management interface was not refused:\n{refused}"
        ));
    }
    golden("tools-switch-uplink-refused", &refused)?;
    g.type_keys(key::F12)?;
    g.type_keys(key::F12)?;
    titled(g, SWITCHES, deadline)?;
    let set = set_uplink(g, 0, 1, &uplink_mac(0), "10", deadline)?;
    if !set.contains("has its uplink") {
        return Err(format!("the uplink was not set:\n{set}"));
    }
    let after = channel(g)?
        .status()?
        .fact(release::fact::MANAGEMENT)
        .unwrap_or("")
        .to_string();
    let enslaved = channel(g)?.text("/proc/net/route")?;
    if before.is_empty() || after != before || !enslaved.lines().any(|l| l.starts_with("eth")) {
        return Err(format!(
            "the management network: `{before}`, then `{after}`"
        ));
    }
    power_off(g)?;
    Ok(format!(
        "the management interface {MANAGEMENT_MAC} refused as PRODNET's uplink with QVS4102; \
         another interface taken; the management line `{before}` the same after"
    ))
}

/// Suite 10: live changes: a VLAN added disturbs no traffic; the uplink
/// moved restores outside traffic; an MTU of 9000 passes a full frame
/// and is refused when the uplink cannot carry it.
pub fn live(run: &Run<'_>) -> Result<String, String> {
    let mut rig = Rig::new(3, 1);
    // The third uplink carries 1500 at most.
    rig.nics[2] = Some(1500);
    let (mut net, _) = installed_host(run, &rig, false)?;
    let deadline = Instant::now() + 3 * run.timeout;
    // What is read off the screen is done before a partition runs: its
    // serial output, relayed to the host's serial port by the test hooks,
    // draws over the screen.
    {
        let g = &mut net.guest;
        to_switches(g, deadline)?;
        make(g, "PRODNET", "10", "9000", deadline)?;
        // The third carries 1500: refused for a switch of 9000.
        let refused = set_uplink(g, 0, 1, &uplink_mac(2), "10", deadline)?;
        if !refused.contains("QVS4105") {
            return Err(format!("an uplink of 1500 for MTU 9000:\n{refused}"));
        }
        golden("tools-switch-mtu-refused", &refused)?;
        g.type_keys(key::F12)?;
        g.type_keys(key::F12)?;
        titled(g, SWITCHES, deadline)?;
        let set = set_uplink(g, 0, 1, &uplink_mac(0), "10", deadline)?;
        if !set.contains("has its uplink") {
            return Err(format!("the uplink was not set:\n{set}"));
        }
    }
    let g = &mut net.guest;
    activate(
        g,
        "NETA",
        &guest_spec(0, vec![adapter(5, "PRODNET", 10, &[])]),
    )?;
    let address = guest_address(g, "NETA", run.deadline())?;
    let echo = |net: &Net<'_>, nic, size| {
        net.station
            .ping(nic, 0, address, 10, size, Duration::from_secs(5))
            .map(|(got, _)| got)
            .map_err(|e| e.to_string())
    };
    // A full frame of 9000 bytes, end to end, once the guest answers.
    echo(&net, 0, 56)?;
    // One at a time: a burst of full frames is a queue's room, not the
    // MTU.
    let mut jumbo = 0;
    for _ in 0..10 {
        jumbo += net
            .station
            .ping(0, 0, address, 1, 8972, Duration::from_secs(3))
            .map(|(got, _)| got)
            .map_err(|e| e.to_string())?;
    }
    // A VLAN added, typed at the list while the partition runs: the state
    // says it is done, and nothing stops.
    let step = Duration::from_millis(1500);
    let g = &mut net.guest;
    by_row(g, 0, 1, "2")?;
    std::thread::sleep(step);
    g.type_keys(&format!(
        "{}{}",
        fill(&[Some("10 30"), None], &[40, 4]),
        key::ENTER
    ))?;
    wait_until("VLAN 30 kept", Duration::from_secs(20), || {
        Ok(channel(g)?.text("/data/state/vswitch.toml")?.contains("30"))
    })?;
    let during = echo(&net, 0, 56)?;
    // Moved to the second interface: outside traffic there.
    let g = &mut net.guest;
    let (row, rows) = row_of(g, &uplink_mac(1))?;
    by_row(g, 0, 1, "7")?;
    std::thread::sleep(step);
    by_row(g, row, rows, "1")?;
    std::thread::sleep(step);
    g.type_keys(&format!(
        "{}{}",
        fill(&[Some(""), Some("10")], &[13, 4]),
        key::ENTER
    ))?;
    let second = super::location_of(g, &uplink_mac(1))?;
    wait_until("the uplink moved", Duration::from_secs(20), || {
        Ok(channel(g)?
            .text("/data/state/vswitch.toml")?
            .contains(&second))
    })?;
    std::thread::sleep(Duration::from_secs(2));
    let after_move = echo(&net, 1, 56)?;
    let old_side = echo(&net, 0, 56)?;
    if jumbo != 10 || during != 10 || after_move != 10 || old_side != 0 {
        return Err(format!(
            "echoes: 9000-byte {jumbo}, after the VLAN {during}, after the move {after_move} \
             (and {old_side} on the old side)"
        ));
    }
    Ok(format!(
        "MTU 9000: 10 of 10 echoes of 8972 bytes end to end to {address}; VLAN 30 added: 10 of 10 \
         echoes; the uplink moved to the second interface: 10 of 10 there, 0 on the first; an \
         interface of 1500 refused for it with QVS4105"
    ))
}
