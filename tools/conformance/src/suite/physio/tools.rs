// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 3: the network adapter released at Service Tools option 10 on
//! an installed host, each screen held against its golden file; a
//! partition whose profile names it gets an address through it, and the
//! host no longer has it as a network interface.

use std::time::{Duration, Instant};

use partd::Calls as _;

use super::devices::{NIC_MAC, activate, end, located, passed, profile};
use crate::qemu::PASSED_NIC;
use crate::qemu::term::key;
use crate::suite::base::{Guest, Run, channel};
use crate::suite::install::{
    Kind, PASSWORD, USER, boot, fill, golden, installed, machine, option, power_off, sign_on,
    titled,
};
use crate::suite::network::guest_address;
use crate::suite::partition::manager::{self, partd};
use crate::suite::partition::{HOST_CPUS, HOST_MEMORY_MB, unit};

const IO: &str = "Work with I/O for Partitions";
/// The host's management NIC.
const MANAGEMENT_MAC: &str = "52:54:00:12:34:56";
/// Rows of option 10's list on a page.
const PAGE: usize = 14;

/// The list's rows by location code, as option 10 sorts them.
fn listed(g: &mut Guest<'_>) -> Result<Vec<String>, String> {
    let mut c = partd(g)?;
    let mut all: Vec<String> = c
        .devices()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|d| d.location)
        .chain(
            c.tape_drives()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|d| d.location),
        )
        .collect();
    all.sort();
    Ok(all)
}

/// Bring `location`'s row onto the page and type `n` by it.
fn by(g: &mut Guest<'_>, location: &str, n: &str, deadline: Instant) -> Result<(), String> {
    let all = listed(g)?;
    let at = all
        .iter()
        .position(|l| l == location)
        .ok_or_else(|| format!("{location} is not listed: {all:?}"))?;
    for _ in 0..at / PAGE {
        g.type_keys(key::F8)?;
        std::thread::sleep(Duration::from_millis(500));
    }
    g.drain();
    g.screen(&format!("{location} on the page"), deadline, |grid| {
        grid.contains(location)
    })?;
    let on_page = (all.len() - at / PAGE * PAGE).min(PAGE);
    let mut values: Vec<Option<&str>> = vec![None; on_page];
    values[at % PAGE] = Some(n);
    g.type_keys(&format!(
        "{}{}",
        fill(&values, &vec![1; on_page]),
        key::ENTER
    ))
}

/// The host's network interfaces, by MAC address.
fn macs(g: &mut Guest<'_>) -> Result<Vec<String>, String> {
    let names = channel(g)?.ls("/sys/class/net")?;
    let mut out = Vec::new();
    for n in names {
        let mac = channel(g)?
            .text(&format!("/sys/class/net/{n}/address"))
            .unwrap_or_default();
        out.push(mac.trim().to_ascii_lowercase());
    }
    Ok(out)
}

/// Suite 3.
pub fn service_tools(run: &Run<'_>) -> Result<String, String> {
    crate::suite::partition::fresh_host(run)?;
    let disks = installed(run, Kind::Single)?;
    let unit = unit(run, run.disk)?;
    let mut l = machine(run, &[Some(disks[0].as_path()), Some(unit.as_path())]);
    l.keys = true;
    l.memory_mb = HOST_MEMORY_MB;
    l.cpus = HOST_CPUS;
    l.prealloc = true;
    l.iommu = true;
    l.nic = Some(MANAGEMENT_MAC.into());
    l.passed = passed(run)?;
    let mut guest = boot(run, &l)?;
    let g = &mut guest;
    let deadline = Instant::now() + 3 * run.timeout;
    crate::suite::network::suites::wait_until(
        "the partition manager",
        Duration::from_secs(60),
        || Ok(partd(g)?.list().is_ok()),
    )?;
    let nic = located(g, PASSED_NIC, 0)?;
    if !macs(g)?.contains(&NIC_MAC.to_string()) {
        return Err(format!("{NIC_MAC} is not the host's before its release"));
    }
    sign_on(g, USER, PASSWORD, deadline)?;
    let list = option(g, 10, IO, deadline)?;
    golden("tools-io", &list)?;
    by(g, &nic, "5", deadline)?;
    let shown = titled(g, "Display I/O for Partitions", deadline)?;
    golden("tools-io-display", &shown)?;
    g.type_keys(key::ENTER)?;
    titled(g, IO, deadline)?;
    by(g, &nic, "2", deadline)?;
    let confirm = titled(g, "Confirm Release for Partitions", deadline)?;
    golden("tools-io-release", &confirm)?;
    g.type_keys(&format!("{nic}{}", key::ENTER))?;
    g.drain();
    let released = g.screen("the adapter released", deadline, |grid| {
        grid.row(1).trim() == IO && grid.contains("released")
    })?;
    golden("tools-io-released", &released)?;
    if macs(g)?.contains(&NIC_MAC.to_string()) {
        return Err(format!(
            "{NIC_MAC} is still a network interface of the host"
        ));
    }
    // PID 1 writes the inventory again when the release is posted.
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let inventory = channel(g)?.text("/data/state/inventory.toml")?;
        if !inventory.to_ascii_lowercase().contains(NIC_MAC) {
            break;
        }
        if Instant::now() >= until {
            return Err(format!("the inventory still names {NIC_MAC}"));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    let id = manager::partition(g, "NET")?;
    activate(g, id, profile("NET", &[(&nic, true)]))?;
    let address = guest_address(g, "NET", deadline + Duration::from_mins(3))?;
    end(g, id)?;
    power_off(g)?;
    Ok(format!(
        "option 10, 5 and 2 by {nic}, its location typed; golden files tools-io, \
         tools-io-display, tools-io-release, tools-io-released; {NIC_MAC} no longer among \
         the host's interfaces nor in its inventory; partition NET took {address} through it"
    ))
}
