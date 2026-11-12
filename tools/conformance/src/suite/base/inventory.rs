// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 5: inventory. QEMU is started with a known topology and
//! `inventory.toml` must match it.

use state::Inventory;

use release::fact;

use super::{DHCP_ADDRESS, MAC, Run, STATE_DIR, booted, channel};
use crate::qemu::image::blank;
use crate::qemu::{DISK_SERIAL, Drive, Launch};

/// Sockets, cores a socket, threads a core.
const TOPOLOGY: (u32, u32, u32) = (2, 2, 1);
/// Guest memory, MiB.
const MEMORY_MB: u32 = 2048;
/// The second disk: its serial number and its size.
const SECOND_SERIAL: &str = "QSVSHD1";
const SECOND_BYTES: u64 = 64 << 20;

fn check(what: &str, ok: bool, found: impl std::fmt::Debug) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(format!("inventory: {what}: found {found:?}"))
    }
}

/// The processors and the memory are what QEMU was given.
fn machine(inv: &Inventory, kvm: bool) -> Result<(), String> {
    let (sockets, cores, threads) = TOPOLOGY;
    let p = &inv.processors;
    check(
        "processors",
        (p.sockets, p.cores, p.threads) == (sockets, sockets * cores, sockets * cores * threads),
        p,
    )?;
    check("the processor's model", !p.model.is_empty(), p)?;
    if kvm {
        check(
            "VMX or SVM under KVM",
            p.virtualization == "VMX" || p.virtualization == "SVM",
            p,
        )?;
    }
    // The kernel keeps a little of what the machine has for itself.
    let memory = u64::from(MEMORY_MB) << 20;
    check(
        "memory",
        inv.memory.total_bytes <= memory && inv.memory.total_bytes >= memory / 10 * 8,
        &inv.memory,
    )?;
    check("2 MB huge pages", inv.memory.huge_2m, &inv.memory)
}

/// The disks, the interface and the PCI devices are what QEMU was
/// given, each with its location code.
fn devices(inv: &Inventory, image_bytes: u64) -> Result<(), String> {
    let disks: Vec<(&str, u64, &str)> = inv
        .disks
        .iter()
        .map(|d| (d.serial.as_str(), d.bytes, d.transport.as_str()))
        .collect();
    let wanted = [
        (DISK_SERIAL, image_bytes, "virtio"),
        (SECOND_SERIAL, SECOND_BYTES, "virtio"),
    ];
    check(
        "two disks with their serials and sizes",
        disks == wanted,
        &inv.disks,
    )?;
    check(
        "the disks' model and location",
        inv.disks
            .iter()
            .all(|d| d.model == "virtio-blk" && d.location.starts_with("P0000-B00-D")),
        &inv.disks,
    )?;
    let [nic] = inv.interfaces.as_slice() else {
        return check("one network interface", false, &inv.interfaces);
    };
    check(
        "the interface's MAC, driver and carrier",
        nic.mac == MAC && nic.driver == "virtio_net" && nic.carrier,
        nic,
    )?;
    // The NIC is a PCI device, and its interface's code starts with it.
    let Some(pci) = inv
        .pci
        .iter()
        .find(|d| d.vendor == "1af4" && d.class.starts_with("02"))
    else {
        return check(
            "a virtio network device among the PCI devices",
            false,
            &inv.pci,
        );
    };
    check(
        "the interface's location is its PCI device's",
        nic.location.starts_with(&pci.location),
        (&nic.location, &pci.location),
    )?;
    let located: Vec<&String> = inv
        .disks
        .iter()
        .map(|d| &d.location)
        .chain(inv.interfaces.iter().map(|i| &i.location))
        .chain(inv.pci.iter().map(|d| &d.location))
        .collect();
    check(
        "every device has a location code",
        located.iter().all(|l| !l.is_empty()),
        &located,
    )?;
    check(
        "the system's serial number",
        !inv.system.serial.is_empty(),
        &inv.system,
    )
}

/// The suite.
pub fn inventory(run: &Run<'_>) -> Result<String, String> {
    let second = run.file("second.img");
    blank(&second, SECOND_BYTES)?;
    let launch = Launch {
        topology: Some(TOPOLOGY),
        memory_mb: MEMORY_MB,
        nic: Some(MAC.to_string()),
        extra: vec![Drive {
            path: second,
            serial: SECOND_SERIAL.into(),
        }],
        ..run.launch()
    };
    let mut guest = booted(run, &launch)?;
    // The interface's carrier is in the inventory once it is up: PID 1
    // writes the file again when the management line changes.
    let deadline = run.deadline();
    let channel = channel(&mut guest)?;
    channel.until("the management address", deadline, |s| {
        s.fact(fact::MANAGEMENT)
            .is_some_and(|m| m.starts_with(DHCP_ADDRESS))
    })?;
    let text = channel.text(&format!("{STATE_DIR}/inventory.toml"))?;
    let inv: Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
    machine(&inv, launch.kvm)?;
    let image = std::fs::metadata(run.disk).map_err(|e| e.to_string())?;
    devices(&inv, image.len())?;
    let (p, nic) = (&inv.processors, &inv.interfaces[0]);
    Ok(format!(
        "{} sockets, {} cores, {} threads, {}; {} MiB; disks {} and {}; {} {} at {}; \
         {} PCI devices; serial {} ({})",
        p.sockets,
        p.cores,
        p.threads,
        p.virtualization,
        inv.memory.total_bytes >> 20,
        inv.disks[0].serial,
        inv.disks[1].serial,
        nic.name,
        nic.mac,
        nic.location,
        inv.pci.len(),
        inv.system.serial,
        inv.system.serial_source
    ))
}
