// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The fixture appliance, `QSDEMO`: what another appliance takes of
//! the base crates, used from a git dependency. Each part prints one
//! `QSDEMO <part>: ...` line when it holds, and the run ends with an
//! error when one does not.

use std::path::Path;

/// QSDEMO's two network roles.
const ROLES: &[&str] = &["public", "private"];

/// Its `network.toml`: the public interface by DHCP, the private one on
/// a VLAN of its own link, carrying neither the route, the name servers
/// nor the time.
const NETWORK: &str = "schema_version = 2\n\
    [[interface]]\nrole = \"public\"\n\
    [[interface]]\nrole = \"private\"\ndevice = \"eth1\"\nvlan = 20\nmode = \"static\"\n\
    address = \"10.1.0.1/24\"\ncarries_route = false\ncarries_dns = false\ncarries_time = false\n";

fn processor() -> Result<String, String> {
    let all = processor::lacking_from("avx2 adx aes pclmulqdq");
    if !all.is_empty() {
        return Err(format!("a processor with every feature lacks {all:?}"));
    }
    let lacking = processor::lacking();
    Ok(format!(
        "{} features checked; this processor lacks {}",
        processor::NEEDED.len(),
        if lacking.is_empty() {
            "none".to_string()
        } else {
            lacking.join(", ")
        }
    ))
}

fn qemu() -> Result<String, String> {
    let launch = qemu::Launch {
        qemu: "qemu-system-x86_64".into(),
        firmware: "/fw/OVMF_CODE_4M.fd".into(),
        disk: Some("qsdemo.img".into()),
        disk_serial: "QSDEMOHD0".into(),
        cdrom: Some("qsdemo.iso".into()),
        nic: Some("52:54:00:00:00:01".into()),
        forwards: vec![(40_000, 443)],
        memory_mb: 512,
        cpus: 1,
        snapshot: true,
        ..qemu::Launch::default()
    };
    let words: Vec<String> = qemu::args(&launch)
        .iter()
        .map(|w| w.to_string_lossy().into_owned())
        .collect();
    for want in [
        "virtio-blk-pci,drive=hd,serial=QSDEMOHD0",
        "scsi-cd,drive=cd,bus=cdscsi.0",
        "user,id=net0,hostfwd=tcp:127.0.0.1:40000-10.0.2.15:443",
    ] {
        if !words.iter().any(|w| w == want) {
            return Err(format!("`{want}` is not on the command line: {words:?}"));
        }
    }
    // A store with one boot entry, read back.
    let store = qemu::vars::with_boot_entry(&empty_store(), b"entry")
        .map_err(|e| format!("the variable store: {e}"))?;
    let names: Vec<String> = qemu::vars::variables(&store)?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    if names != ["Boot0000", "BootOrder"] {
        return Err(format!("the store has {names:?}"));
    }
    let missing = qemu::firmware::locate_vars(Path::new("/nonexistent/CODE.fd"), "QSDEMO_OVMF_VARS");
    match missing {
        Err(e) if e.to_string().contains("QSDEMO_OVMF_VARS") => {}
        other => return Err(format!("a missing store: {other:?}")),
    }
    Ok(format!(
        "{} arguments, the CD-ROM on virtio-scsi; a store with {}",
        words.len(),
        names.join(" and ")
    ))
}

/// An erased variable store: a firmware volume header with an
/// authenticated variable store and nothing in it.
fn empty_store() -> Vec<u8> {
    const LEN: u32 = 0x2_0000;
    let mut s = vec![0xFF_u8; 0x2_0000];
    s[..0x48].fill(0);
    // The volume header: its length at 32, `_FVH` at 40, its header
    // length at 48.
    s[32..40].copy_from_slice(&u64::from(LEN).to_le_bytes());
    s[40..44].copy_from_slice(b"_FVH");
    s[48..50].copy_from_slice(&0x48u16.to_le_bytes());
    // The variable store's header: its GUID, size, formatted and healthy.
    let guid = [
        0x78, 0x2c, 0xf3, 0xaa, 0x7b, 0x94, 0x9a, 0x43, 0xa1, 0x80, 0x2e, 0x14, 0x4e, 0xc3, 0x77,
        0x92,
    ];
    s[0x48..0x58].copy_from_slice(&guid);
    s[0x58..0x5C].copy_from_slice(&(LEN - 0x48).to_le_bytes());
    s[0x5C] = 0x5A;
    s[0x5D] = 0xFE;
    s[0x5E..0x64].fill(0);
    s
}

fn inventory() -> Result<String, String> {
    let roots = inventory::Roots::default();
    let inv = inventory::read(&roots, None);
    let text = state::store::render(&inv).map_err(|e| e.to_string())?;
    let back: state::Inventory = state::store::parse(&text).map_err(|e| e.to_string())?;
    if back != inv {
        return Err("inventory.toml does not read back".into());
    }
    if inv.processors.threads == 0 || inventory::threads(&roots).is_empty() {
        return Err("no processor was read".into());
    }
    Ok(format!(
        "{} threads, {} MiB, {} disks, {} interfaces, {} PCI devices; serial from {}",
        inv.processors.threads,
        inventory::memory_mib(&roots),
        inv.disks.len(),
        inv.interfaces.len(),
        inv.pci.len(),
        inv.system.serial_source
    ))
}

fn network() -> Result<String, String> {
    let network: state::Network = state::store::parse(NETWORK).map_err(|e| e.to_string())?;
    netd::manage::check_roles(&network, ROLES)?;
    let mut moved = network.clone();
    moved.interfaces[1].vlan = Some(30);
    let touched = netd::trial::changed(&network, &moved);
    if touched != ["private"] {
        return Err(format!("a change of the private VLAN touches {touched:?}"));
    }
    let version_1: state::Network =
        state::store::parse("schema_version = 1\nmode = \"static\"\naddress = \"192.0.2.10/24\"\n")
            .map_err(|e| e.to_string())?;
    netd::manage::check_roles(&version_1, ROLES)?;
    Ok(format!(
        "{} interfaces, roles {}; a change of the private one touches it alone; \
         a version 1 file is {} interface",
        network.interfaces.len(),
        ROLES.join(" and "),
        version_1.interfaces.len()
    ))
}

/// One part: what it says when it held, or why it did not.
type Part = fn() -> Result<String, String>;

fn main() -> Result<(), String> {
    let parts: [(&str, Part); 4] = [
        ("processor", processor),
        ("qemu", qemu),
        ("inventory", inventory),
        ("netd", network),
    ];
    for (name, part) in parts {
        let line = part().map_err(|e| format!("QSDEMO {name} FAILED: {e}"))?;
        println!("QSDEMO {name}: {line}");
    }
    Ok(())
}
