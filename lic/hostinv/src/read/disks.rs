// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The disks, from `/sys/class/block`: whole disks only, so not a
//! partition, a loop, RAM, mapped or RAID device, an optical drive or
//! an empty removable drive.

use std::fs;
use std::path::{Path, PathBuf};

use state::doc::inventory::Disk;

use super::place::chain;
use crate::location::{PciAddress, Slot, Unit, unit_code};
use crate::sysfs::{Node, names, number, numbered, resolve, text};

/// Bytes in the sector `size` counts in, whatever the disk's own.
const SECTOR: u64 = 512;

/// How far below a PCI function its SCSI hosts are looked for.
const HOST_DEPTH: u32 = 8;

/// The disks, by name.
pub(crate) fn read(sys: &Path, slots: &[Slot]) -> Vec<Disk> {
    let class = sys.join("class/block");
    names(&class)
        .into_iter()
        .filter(|name| !skipped(name))
        .filter_map(|name| disk(sys, slots, &class.join(&name), name))
        .collect()
}

/// Whether a name is a kind of block device that is never a disk.
fn skipped(name: &str) -> bool {
    name.starts_with("dm-")
        || ["loop", "ram", "zram", "md", "sr"]
            .iter()
            .any(|prefix| numbered(name, prefix).is_some())
}

/// One disk; `None` for a partition, a hidden path of a multipath disk
/// or an empty removable drive.
fn disk(sys: &Path, slots: &[Slot], dir: &Path, name: String) -> Option<Disk> {
    let flag = |file: &str| text(dir.join(file)).as_deref() == Some("1");
    if !dir.is_dir() || dir.join("partition").exists() || flag("hidden") {
        return None;
    }
    let sectors: u64 = number(dir.join("size")).unwrap_or(0);
    if sectors == 0 && flag("removable") {
        return None;
    }
    let node = controller(sys, resolve(sys, dir));
    let transport = transport(&node.parts);
    let chain = chain(&node.parts);
    let unit = if transport == "nvme" {
        Unit::Namespace(namespace(dir, &name))
    } else {
        scsi_unit(&node)
    };
    Some(Disk {
        model: model(dir, transport),
        serial: serial(dir),
        bytes: sectors.saturating_mul(SECTOR),
        rotational: flag("queue/rotational"),
        transport: transport.to_string(),
        location: unit_code(&chain, slots, &name, unit),
        name,
    })
}

/// A multipath disk sits under its virtual subsystem, which links to
/// its controllers; the lowest one says where it is.
fn controller(sys: &Path, node: Node) -> Node {
    let subsystem = node
        .path
        .parent()
        .filter(|dir| {
            let name = dir.file_name().map(|n| n.to_string_lossy());
            name.is_some_and(|n| n.starts_with("nvme-subsys"))
        })
        .map(Path::to_path_buf);
    let Some(subsystem) = subsystem else {
        return node;
    };
    let mut controllers: Vec<u32> = names(&subsystem)
        .iter()
        .filter_map(|name| numbered(name, "nvme"))
        .collect();
    controllers.sort_unstable();
    match controllers.first() {
        Some(first) => resolve(sys, &subsystem.join(format!("nvme{first}"))),
        None => node,
    }
}

/// The transport, from the components of the disk's sysfs path.
fn transport(parts: &[String]) -> &'static str {
    let any = |test: &dyn Fn(&str) -> bool| parts.iter().any(|part| test(part));
    let sas = ["port-", "end_device-", "expander-"];
    if any(&|p| p == "nvme" || p.starts_with("nvme-subsys") || numbered(p, "nvme").is_some()) {
        "nvme"
    } else if any(&|p| numbered(p, "usb").is_some()) {
        "usb"
    } else if any(&|p| numbered(p, "virtio").is_some()) {
        "virtio"
    } else if any(&|p| numbered(p, "ata").is_some()) {
        "sata"
    } else if any(&|p| sas.iter().any(|prefix| p.starts_with(prefix))) {
        "sas"
    } else if any(&|p| numbered(p, "host").is_some()) {
        "scsi"
    } else {
        "unknown"
    }
}

/// What a virtio block device is called: it has no model of its own.
const VIRTIO_BLOCK: &str = "virtio-blk";

/// The model, with the vendor before it unless that is only `ATA`. A
/// virtio block device has no model, and its `vendor` is the virtio
/// device's vendor ID, not a name.
fn model(dir: &Path, transport: &str) -> String {
    let model = text(dir.join("device/model")).unwrap_or_default();
    if model.is_empty() && transport == "virtio" {
        return VIRTIO_BLOCK.to_string();
    }
    match text(dir.join("device/vendor")) {
        Some(vendor) if !vendor.is_empty() && vendor != "ATA" => {
            format!("{vendor} {model}").trim().to_string()
        }
        _ => model,
    }
}

/// The serial number, from the first node that has one.
fn serial(dir: &Path) -> String {
    let found = |serial: Option<String>| serial.filter(|s| !s.is_empty());
    found(text(dir.join("device/serial")))
        .or_else(|| found(text(dir.join("serial"))))
        .or_else(|| found(unit_serial(dir)))
        .or_else(|| found(text(dir.join("device/wwid"))))
        .unwrap_or_default()
}

/// The SCSI unit serial number page: a four-byte header whose last two
/// bytes are the length, then the serial in ASCII.
fn unit_serial(dir: &Path) -> Option<String> {
    let page = fs::read(dir.join("device/vpd_pg80")).ok()?;
    let length = usize::from(u16::from_be_bytes([*page.get(2)?, *page.get(3)?]));
    let data = page.get(4..)?;
    let data = &data[..length.min(data.len())];
    let serial: String = data
        .iter()
        .filter(|b| b.is_ascii_graphic() || **b == b' ')
        .map(|b| char::from(*b))
        .collect();
    Some(serial.trim().to_string())
}

/// The namespace number: the `nsid` node, or the name's own.
fn namespace(dir: &Path, name: &str) -> u32 {
    number(dir.join("nsid"))
        .or_else(|| name.rsplit_once('n')?.1.parse().ok())
        .unwrap_or(0)
}

/// The SCSI address in the path, its host counted among the hosts of
/// the disk's PCI function; `Whole` when the path has none.
fn scsi_unit(node: &Node) -> Unit {
    let address = node.parts.iter().rev().find_map(|part| {
        let mut fields = part.split(':');
        let host: u32 = fields.next()?.parse().ok()?;
        let channel: u32 = fields.next()?.parse().ok()?;
        let target: u32 = fields.next()?.parse().ok()?;
        let lun: u64 = fields.next()?.parse().ok()?;
        fields
            .next()
            .is_none()
            .then_some((host, channel, target, lun))
    });
    let Some((host, channel, target, lun)) = address else {
        return Unit::Whole;
    };
    Unit::Scsi {
        host: function_dir(node).map_or(host, |dir| rank(&dir, host)),
        channel,
        target,
        lun,
    }
}

/// The directory of the last PCI function in the path.
fn function_dir(node: &Node) -> Option<PathBuf> {
    let last = node
        .parts
        .iter()
        .rposition(|part| PciAddress::parse(part).is_some())?;
    Some(
        node.parts[..=last]
            .iter()
            .fold(node.sys.clone(), |dir, part| dir.join(part)),
    )
}

/// Where `host` comes among the SCSI hosts below `dir`, from 0; `host`
/// itself when it is not found there.
fn rank(dir: &Path, host: u32) -> u32 {
    let mut hosts = Vec::new();
    hosts_below(dir, HOST_DEPTH, &mut hosts);
    hosts.sort_unstable();
    hosts
        .iter()
        .position(|h| *h == host)
        .and_then(|at| u32::try_from(at).ok())
        .unwrap_or(host)
}

/// The numbers of the `host<n>` directories below `dir`. Links are not
/// followed, and a host is not looked into.
fn hosts_below(dir: &Path, depth: u32, hosts: &mut Vec<u32>) {
    if depth == 0 {
        return;
    }
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        match numbered(&entry.file_name().to_string_lossy(), "host") {
            Some(host) => hosts.push(host),
            None => hosts_below(&entry.path(), depth - 1, hosts),
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fixture::Tree;
    use crate::location::normalise;

    /// A block device at `path` (below `sys/devices`) with a class link.
    fn block(tree: &Tree, path: &str, sectors: u64, rotational: bool) -> String {
        let dir = format!("sys/devices/{path}");
        tree.file(&format!("{dir}/size"), format!("{sectors}\n"));
        let spins = if rotational { "1\n" } else { "0\n" };
        tree.file(&format!("{dir}/queue/rotational"), spins);
        let name = path.rsplit('/').next().unwrap();
        tree.link(&format!("sys/class/block/{name}"), &dir);
        dir
    }

    fn parts(path: &str) -> Vec<String> {
        path.split('/').map(str::to_string).collect()
    }

    #[test]
    fn a_virtio_disk_is_read_with_its_serial() {
        let tree = Tree::new("disks-virtio");
        let path = "pci0000:00/0000:00:03.0/virtio1/block/vda";
        let vda = block(&tree, path, 2_097_152, false);
        tree.file(&format!("{vda}/serial"), "DISK0\n");
        tree.link(
            &format!("{vda}/device"),
            "sys/devices/pci0000:00/0000:00:03.0/virtio1",
        );
        // The virtio device's vendor ID, which is no model.
        tree.file(
            "sys/devices/pci0000:00/0000:00:03.0/virtio1/vendor",
            "0x1af4\n",
        );
        // A partition of it is not a disk.
        let part = block(&tree, &format!("{path}/vda1"), 2048, false);
        tree.file(&format!("{part}/partition"), "1\n");
        let found = read(&tree.sys(), &[]);
        assert_eq!(
            found,
            [Disk {
                name: "vda".into(),
                model: "virtio-blk".into(),
                serial: "DISK0".into(),
                bytes: 1 << 30,
                rotational: false,
                transport: "virtio".into(),
                location: "P0000-B00-D03-F0".into(),
            }]
        );
    }

    #[test]
    fn an_nvme_disk_is_read_with_its_namespace() {
        let tree = Tree::new("disks-nvme");
        let ctrl = "pci0000:00/0000:00:1c.0/0000:03:00.0/nvme/nvme0";
        let disk = block(&tree, &format!("{ctrl}/nvme0n1"), 1_000_215_216, false);
        tree.file(
            &format!("sys/devices/{ctrl}/model"),
            "Example SSD 500GB  \n",
        );
        tree.file(&format!("sys/devices/{ctrl}/serial"), "S4EVNX0M123456 \n");
        tree.file(&format!("{disk}/nsid"), "1\n");
        tree.link(&format!("{disk}/device"), &format!("sys/devices/{ctrl}"));
        let slots = [Slot {
            designation: normalise("M.2 Slot 1"),
            address: PciAddress::parse("0000:00:1c.0").unwrap(),
            bridge: true,
        }];
        let found = read(&tree.sys(), &slots);
        assert_eq!(
            found,
            [Disk {
                name: "nvme0n1".into(),
                model: "Example SSD 500GB".into(),
                serial: "S4EVNX0M123456".into(),
                bytes: 1_000_215_216 * 512,
                rotational: false,
                transport: "nvme".into(),
                location: "SM-2-SLOT-1-N1".into(),
            }]
        );
        assert_eq!(read(&tree.sys(), &[])[0].location, "P0000-B03-D00-F0-N1");
    }

    #[test]
    fn a_multipath_nvme_disk_is_placed_by_its_first_controller() {
        let tree = Tree::new("disks-multipath");
        let subsys = "virtual/nvme-subsystem/nvme-subsys0";
        let head = block(&tree, &format!("{subsys}/nvme0n2"), 4096, false);
        tree.file(&format!("sys/devices/{subsys}/model"), "Example SSD\n");
        tree.file(&format!("sys/devices/{subsys}/serial"), "SUBSYS1\n");
        tree.link(&format!("{head}/device"), &format!("sys/devices/{subsys}"));
        let ctrl = "sys/devices/pci0000:00/0000:04:00.0/nvme/nvme0";
        tree.dir(ctrl);
        tree.link(&format!("sys/devices/{subsys}/nvme0"), ctrl);
        // The path through one controller is hidden.
        let hidden = block(
            &tree,
            "pci0000:00/0000:04:00.0/nvme/nvme0/nvme0c0n2",
            0,
            false,
        );
        tree.file(&format!("{hidden}/hidden"), "1\n");
        let found = read(&tree.sys(), &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "nvme0n2");
        assert_eq!(found[0].transport, "nvme");
        assert_eq!(found[0].serial, "SUBSYS1");
        // No `nsid` node: the name's own number.
        assert_eq!(found[0].location, "P0000-B04-D00-F0-N2");
    }

    #[test]
    fn a_scsi_disk_is_read_with_its_unit_serial_page() {
        let tree = Tree::new("disks-scsi");
        let hba = "pci0000:00/0000:00:1f.2";
        let unit = format!("{hba}/ata3/host2/target2:0:0/2:0:0:0");
        // Another port of the adapter, and another adapter's host.
        tree.dir(&format!("sys/devices/{hba}/ata1/host0"));
        tree.dir("sys/devices/pci0000:00/0000:00:1f.5/host1");
        let sda = block(&tree, &format!("{unit}/block/sda"), 7_814_037_168, true);
        tree.link(&format!("{sda}/device"), &format!("sys/devices/{unit}"));
        tree.file(&format!("sys/devices/{unit}/vendor"), "ATA     \n");
        tree.file(&format!("sys/devices/{unit}/model"), "EXAMPLE HDD 4TB \n");
        let mut page = vec![0x00, 0x80, 0x00, 0x0c];
        page.extend(b"    WD-12345");
        page.extend(b"junk past the length");
        tree.file(&format!("sys/devices/{unit}/vpd_pg80"), &page);
        tree.file(
            &format!("sys/devices/{unit}/wwid"),
            "naa.50014ee2b5c1d2e3\n",
        );
        let found = read(&tree.sys(), &[]);
        assert_eq!(
            found,
            [Disk {
                name: "sda".into(),
                model: "EXAMPLE HDD 4TB".into(),
                serial: "WD-12345".into(),
                bytes: 7_814_037_168 * 512,
                rotational: true,
                transport: "sata".into(),
                // Host 2 is the second host of its own adapter.
                location: "P0000-B00-D1F-F2-T1.0.0.0".into(),
            }]
        );
    }

    #[test]
    fn a_disk_with_no_serial_page_falls_back_to_its_wwid() {
        let tree = Tree::new("disks-wwid");
        let unit = "pci0000:00/0000:02:00.0/host0/port-0:0/end_device-0:0/target0:0:5/0:0:5:0";
        let sdb = block(&tree, &format!("{unit}/block/sdb"), 1024, true);
        tree.link(&format!("{sdb}/device"), &format!("sys/devices/{unit}"));
        tree.file(&format!("sys/devices/{unit}/vendor"), "EXAMPLE\n");
        tree.file(&format!("sys/devices/{unit}/model"), "ST4000\n");
        tree.file(
            &format!("sys/devices/{unit}/wwid"),
            "naa.5000c500a1b2c3d4\n",
        );
        let found = read(&tree.sys(), &[]);
        assert_eq!(found[0].model, "EXAMPLE ST4000");
        assert_eq!(found[0].serial, "naa.5000c500a1b2c3d4");
        assert_eq!(found[0].transport, "sas");
        assert_eq!(found[0].location, "P0000-B02-D00-F0-T0.0.5.0");
    }

    #[test]
    fn devices_that_are_not_disks_are_left_out() {
        let tree = Tree::new("disks-skipped");
        for name in ["loop0", "ram1", "zram0", "dm-0", "md127", "sr0"] {
            block(&tree, &format!("virtual/block/{name}"), 1024, false);
        }
        // A card reader with no card in it.
        let reader = block(
            &tree,
            "pci0000:00/0000:00:14.0/usb1/1-2/host6/6:0:0:0/block/sdc",
            0,
            false,
        );
        tree.file(&format!("{reader}/removable"), "1\n");
        // A removable disk with media, and a disk named like none of the above.
        let stick = block(
            &tree,
            "pci0000:00/0000:00:14.0/usb1/1-3/host7/7:0:0:0/block/sdd",
            2048,
            false,
        );
        tree.file(&format!("{stick}/removable"), "1\n");
        block(&tree, "platform/soc/mmc0/mmcblk0", 4096, false);
        let found = read(&tree.sys(), &[]);
        let names: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["mmcblk0", "sdd"]);
        assert_eq!(found[0].transport, "unknown");
        assert_eq!(found[0].location, "NOPCI-MMCBLK0");
        assert_eq!(found[1].transport, "usb");
        assert_eq!(found[1].location, "P0000-B00-D14-F0-T1.0.0.0");
    }

    #[test]
    fn the_transport_comes_from_the_device_path() {
        for (path, expected) in [
            ("devices/pci0000:00/0000:03:00.0/nvme/nvme0/nvme0n1", "nvme"),
            (
                "devices/virtual/nvme-subsystem/nvme-subsys0/nvme0n1",
                "nvme",
            ),
            (
                "devices/pci0000:00/0000:00:03.0/virtio1/block/vda",
                "virtio",
            ),
            (
                "devices/pci0000:00/0000:00:05.0/virtio2/host0/target0:0:0/0:0:0:0/block/sda",
                "virtio",
            ),
            (
                "devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host4/target4:0:0/4:0:0:0/block/sdb",
                "usb",
            ),
            (
                "devices/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/0:0:0:0/block/sda",
                "sata",
            ),
            (
                "devices/pci0000:00/0000:02:00.0/host0/port-0:0/end_device-0:0/target0:0:0/0:0:0:0/block/sda",
                "sas",
            ),
            (
                "devices/pci0000:00/0000:02:00.0/host0/target0:0:0/0:0:0:0/block/sda",
                "scsi",
            ),
            ("devices/platform/soc/mmc0/mmcblk0", "unknown"),
            ("", "unknown"),
        ] {
            assert_eq!(transport(&parts(path)), expected, "{path}");
        }
    }

    #[test]
    fn a_damaged_serial_page_gives_no_serial() {
        let tree = Tree::new("disks-page");
        let dir = tree.sys().join("d");
        tree.file("sys/d/device/vpd_pg80", [0x00, 0x80, 0x00]);
        assert_eq!(unit_serial(&dir), None);
        assert_eq!(serial(&dir), "");
        // A length past the end takes what there is.
        tree.file(
            "sys/d/device/vpd_pg80",
            [0x00, 0x80, 0x00, 0x20, b'A', b'B', 0x00],
        );
        assert_eq!(unit_serial(&dir).as_deref(), Some("AB"));
        assert_eq!(unit_serial(&tree.sys().join("missing")), None);
    }
}
