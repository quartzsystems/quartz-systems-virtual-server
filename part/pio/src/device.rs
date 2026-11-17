// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's PCI devices as physical I/O sees them. A device can be
//! released for partitions only when every safety rule holds for it and
//! for every other device of its isolation group, and none of them is
//! the host's: the IOMMU translates and remaps interrupts, each can be
//! reset alone, each signals message interrupts. No setting relaxes a
//! rule: there is none to set.

use qvsmsg::Issued;
use state::doc::inventory::{Iommu, Pci};

use crate::released::Released;

/// What the host uses a device for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Use {
    /// A controller of a load-source disk.
    LoadSource,
    /// A controller of a storage pool's disk.
    Pool {
        /// The pool.
        pool: String,
    },
    /// The management interface's adapter.
    Management,
    /// An adapter of a switch's uplink.
    Uplink {
        /// The switch.
        switch: String,
    },
    /// A display adapter: the console's, and no partition's in V1.
    Display,
    /// A bridge: the host's PCI plumbing.
    Bridge,
}
ipc::wire_enum!(Use {
    0 => LoadSource,
    1 => Pool { pool },
    2 => Management,
    3 => Uplink { switch },
    4 => Display,
    5 => Bridge,
});

impl Use {
    /// As a person reads it.
    pub fn words(&self) -> String {
        match self {
            Use::LoadSource => "a load-source disk's controller".into(),
            Use::Pool { pool } => format!("a disk controller of storage pool {pool}"),
            Use::Management => "the management interface".into(),
            Use::Uplink { switch } => format!("an uplink of switch {switch}"),
            Use::Display => "the console's display adapter".into(),
            Use::Bridge => "a PCI bridge".into(),
        }
    }

    /// The same in a list's column: 20 characters but for a long name.
    pub fn short(&self) -> String {
        match self {
            Use::LoadSource => "load source".into(),
            Use::Pool { pool } => format!("pool {pool}"),
            Use::Management => "management".into(),
            Use::Uplink { switch } => format!("uplink {switch}"),
            Use::Display => "display".into(),
            Use::Bridge => "bridge".into(),
        }
    }
}

/// Why a device cannot be given to a partition now. The first five
/// are QSI's `Blocker` values; the rest are the host's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// The host uses it, or a device of its group.
    HostUses,
    /// It is not released.
    NotReleased,
    /// A device of its group is not released.
    GroupNotReleased,
    /// It, or a device of its group, cannot be reset alone.
    NoReset,
    /// The host has no interrupt remapping.
    NoRemapping,
    /// The host's IOMMU does not translate.
    NoIommu,
    /// It, or a device of its group, signals no message interrupt.
    NoMessageInterrupts,
    /// It did not reset and is fenced until the host restarts.
    Fenced,
    /// It is released and not on the host.
    Gone,
    /// Its group is not what it was when it was released.
    GroupChanged,
}
ipc::wire_enum!(Blocker {
    0 => HostUses,
    1 => NotReleased,
    2 => GroupNotReleased,
    3 => NoReset,
    4 => NoRemapping,
    5 => NoIommu,
    6 => NoMessageInterrupts,
    7 => Fenced,
    8 => Gone,
    9 => GroupChanged,
});

/// Who holds a device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Holder {
    /// The partition's ID.
    pub partition: u32,
    /// Its name.
    pub name: String,
    /// The profile it was activated with.
    pub profile: String,
}
ipc::wire_struct!(Holder {
    partition,
    name,
    profile
});

/// What the host uses, by the location codes of the disks and
/// interfaces involved; the controller of each is found from the
/// inventory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostUses {
    /// The load-source disks.
    pub load_source: Vec<String>,
    /// Each pool's disks: the pool and a disk.
    pub pool_members: Vec<(String, String)>,
    /// The management interface, and its bond's members.
    pub management: Vec<String>,
    /// Each switch's uplink interfaces: the switch and an interface.
    pub uplinks: Vec<(String, String)>,
}

/// One device, as Service Tools and a console see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    /// Its location code.
    pub location: String,
    /// Its PCI address; empty when it is not on the host.
    pub address: String,
    /// Vendor, device and class, in hex.
    pub vendor: String,
    /// The device ID.
    pub device: String,
    /// The class code.
    pub class: String,
    /// What it is, in the host's words.
    pub description: String,
    /// Its isolation group.
    pub group: Option<u32>,
    /// The other devices of its group, by location code; bridges are
    /// not given and not listed.
    pub with: Vec<String>,
    /// How it can be reset.
    pub reset: Vec<String>,
    /// The interrupts it signals.
    pub interrupts: Vec<String>,
    /// What the host uses it for.
    pub uses: Vec<Use>,
    /// Whether it is on the host now.
    pub present: bool,
    /// Whether it is released for partitions.
    pub released: bool,
    /// Every reason it cannot be released; empty when it can be.
    pub releasable: Vec<Issued>,
    /// Why it cannot be given now; none when it can be.
    pub blocked: Option<(Blocker, Issued)>,
    /// The partition that holds it.
    pub holder: Option<Holder>,
    /// It did not reset and is not given until the host restarts.
    pub fenced: bool,
}
ipc::wire_struct!(Device {
    location,
    address,
    vendor,
    device,
    class,
    description,
    group,
    with,
    reset,
    interrupts,
    uses,
    present,
    released,
    releasable,
    blocked,
    holder,
    fenced
});

impl Device {
    /// Whether a partition can be given it now.
    pub fn assignable(&self) -> bool {
        self.blocked.is_none() && self.holder.is_none()
    }

    /// Every device of its group, it first.
    pub fn group_locations(&self) -> Vec<String> {
        std::iter::once(self.location.clone())
            .chain(self.with.iter().cloned())
            .collect()
    }
}

/// The facts a device list is made of.
#[derive(Clone, Copy, Debug)]
pub struct Facts<'a> {
    /// The PCI devices, as they are now.
    pub pci: &'a [Pci],
    /// The IOMMU.
    pub iommu: &'a Iommu,
    /// What the host uses.
    pub uses: &'a HostUses,
    /// What is released.
    pub released: &'a Released,
    /// Who holds what, by location code.
    pub holders: &'a [(String, Holder)],
    /// The devices fenced since the IPL, by location code.
    pub fenced: &'a [String],
}

/// Whether a class code is a bridge's (base class 06).
pub fn is_bridge(class: &str) -> bool {
    class.starts_with("06")
}

/// What a class code is, in the host's words.
pub fn kind(class: &str) -> &'static str {
    let base = class.get(..2).unwrap_or("");
    let sub = class.get(..4).unwrap_or("");
    match (base, sub) {
        (_, "0108") => "NVMe controller",
        (_, "0106") => "SATA controller",
        (_, "0107") => "SAS controller",
        (_, "0104") => "RAID controller",
        (_, "0100") => "SCSI controller",
        ("01", _) => "storage controller",
        (_, "0200") => "Ethernet adapter",
        ("02", _) => "network adapter",
        ("03", _) => "display adapter",
        (_, "0c04") => "Fibre Channel adapter",
        (_, "0c03") => "USB controller",
        (_, "0600") => "host bridge",
        ("06", _) => "bridge",
        _ => "PCI device",
    }
}

/// The PCI device a disk or interface of location `code` is on: the one
/// whose code it is, or whose code it begins with a unit's suffix after
/// (`docs/LOCATION.md`). The longest such code.
pub fn controller<'p>(code: &str, pci: &'p [Pci]) -> Option<&'p Pci> {
    pci.iter()
        .filter(|p| !p.location.is_empty())
        .filter(|p| {
            code == p.location
                || code
                    .strip_prefix(&p.location)
                    .is_some_and(|rest| ["-T", "-N", "-E"].iter().any(|u| rest.starts_with(u)))
        })
        .max_by_key(|p| p.location.len())
}

/// The uses of each PCI device, by address.
fn uses_of(f: &Facts<'_>) -> Vec<(String, Vec<Use>)> {
    let mut out: Vec<(String, Vec<Use>)> = f
        .pci
        .iter()
        .map(|p| {
            let mut uses = Vec::new();
            if is_bridge(&p.class) {
                uses.push(Use::Bridge);
            }
            if p.class.starts_with("03") || p.boot_display {
                uses.push(Use::Display);
            }
            (p.address.clone(), uses)
        })
        .collect();
    let mut add = |code: &str, what: Use| {
        if let Some(p) = controller(code, f.pci)
            && let Some((_, uses)) = out.iter_mut().find(|(a, _)| *a == p.address)
            && !uses.contains(&what)
        {
            uses.push(what);
        }
    };
    for disk in &f.uses.load_source {
        add(disk, Use::LoadSource);
    }
    for (pool, disk) in &f.uses.pool_members {
        add(disk, Use::Pool { pool: pool.clone() });
    }
    for interface in &f.uses.management {
        add(interface, Use::Management);
    }
    for (switch, interface) in &f.uses.uplinks {
        add(
            interface,
            Use::Uplink {
                switch: switch.clone(),
            },
        );
    }
    out
}

/// The devices, by location code: every PCI device of the host, and
/// every released one that is not on it now.
pub fn assess(f: &Facts<'_>) -> Vec<Device> {
    let uses = uses_of(f);
    let uses_at = |address: &str| -> Vec<Use> {
        uses.iter()
            .find(|(a, _)| a == address)
            .map(|(_, u)| u.clone())
            .unwrap_or_default()
    };
    let members = |group: Option<u32>, me: &Pci| -> Vec<&Pci> {
        match group {
            Some(g) => f
                .pci
                .iter()
                .filter(|p| p.group == Some(g) && p.address != me.address)
                .collect(),
            None => Vec::new(),
        }
    };
    let mut devices: Vec<Device> = f
        .pci
        .iter()
        .map(|p| {
            let others = members(p.group, p);
            let given_with: Vec<&Pci> = others
                .iter()
                .copied()
                .filter(|o| !is_bridge(&o.class))
                .collect();
            let mut d = Device {
                location: p.location.clone(),
                address: p.address.clone(),
                vendor: p.vendor.clone(),
                device: p.device.clone(),
                class: p.class.clone(),
                description: format!("{} {}:{}", kind(&p.class), p.vendor, p.device),
                group: p.group,
                with: given_with.iter().map(|o| o.location.clone()).collect(),
                reset: p.reset.clone(),
                interrupts: p.interrupts.clone(),
                uses: uses_at(&p.address),
                present: true,
                released: f.released.has(&p.location),
                fenced: f.fenced.contains(&p.location),
                holder: f
                    .holders
                    .iter()
                    .find(|(l, _)| *l == p.location)
                    .map(|(_, h)| h.clone()),
                ..Device::default()
            };
            d.releasable = releasable(f, &d, &given_with, &uses_at);
            d
        })
        .collect();
    for r in &f.released.devices {
        if !devices.iter().any(|d| d.location == r.location) {
            devices.push(Device {
                location: r.location.clone(),
                vendor: r.vendor.clone(),
                device: r.device.clone(),
                description: format!("released device {}:{}", r.vendor, r.device),
                released: true,
                fenced: f.fenced.contains(&r.location),
                ..Device::default()
            });
        }
    }
    for i in 0..devices.len() {
        let blocked = blocked(f, &devices[i], &devices);
        devices[i].blocked = blocked;
    }
    devices.sort_by(|a, b| a.location.cmp(&b.location));
    devices
}

/// Every reason `d` cannot be released, `others` the rest of its group
/// that would go with it.
fn releasable(
    f: &Facts<'_>,
    d: &Device,
    others: &[&Pci],
    uses_at: &dyn Fn(&str) -> Vec<Use>,
) -> Vec<Issued> {
    let mut why = Vec::new();
    if !f.iommu.translation {
        why.push(qvsmsg::NO_IOMMU.issue());
    }
    if !f.iommu.remapping {
        why.push(qvsmsg::NO_REMAPPING.issue());
    }
    if let Some(u) = d.uses.first() {
        why.push(qvsmsg::DEVICE_HOST_USES.with(&[&d.location, &u.words()]));
        return why;
    }
    if d.group.is_none() && f.iommu.translation {
        why.push(qvsmsg::NO_IOMMU.issue());
    }
    // A used endpoint of the group keeps the whole group; its bridges
    // stay the host's and are not in `others`.
    for o in others {
        if let Some(u) = uses_at(&o.address).first() {
            why.push(qvsmsg::DEVICE_GROUP_HOST.with(&[&d.location, &o.location, &u.words()]));
        }
    }
    let mine = (
        d.location.as_str(),
        d.reset.as_slice(),
        d.interrupts.as_slice(),
    );
    let group = std::iter::once(mine).chain(others.iter().map(|o| {
        (
            o.location.as_str(),
            o.reset.as_slice(),
            o.interrupts.as_slice(),
        )
    }));
    for (location, reset, interrupts) in group {
        if reset.is_empty() {
            why.push(qvsmsg::DEVICE_NO_RESET.with(&[location]));
        }
        if !interrupts.iter().any(|i| i == "msi" || i == "msix") {
            why.push(qvsmsg::DEVICE_NO_MSI.with(&[location]));
        }
    }
    why
}

/// The blocker of the first reason it cannot be released.
fn blocker_of(why: &Issued) -> Blocker {
    if why.is(&qvsmsg::NO_REMAPPING) {
        Blocker::NoRemapping
    } else if why.is(&qvsmsg::NO_IOMMU) {
        Blocker::NoIommu
    } else if why.is(&qvsmsg::DEVICE_NO_RESET) {
        Blocker::NoReset
    } else if why.is(&qvsmsg::DEVICE_NO_MSI) {
        Blocker::NoMessageInterrupts
    } else {
        Blocker::HostUses
    }
}

/// Why `d` cannot be given now, among `all`.
fn blocked(f: &Facts<'_>, d: &Device, all: &[Device]) -> Option<(Blocker, Issued)> {
    if !d.present {
        return Some((Blocker::Gone, qvsmsg::DEVICE_GONE.with(&[&d.location])));
    }
    if !d.released {
        if let Some(why) = d.releasable.first() {
            return Some((blocker_of(why), why.clone()));
        }
        return Some((
            Blocker::NotReleased,
            qvsmsg::DEVICE_NOT_RELEASED.with(&[&d.location]),
        ));
    }
    if let Some(changed) = f.released.group_change(&d.location, &d.group_locations()) {
        return Some((
            Blocker::GroupChanged,
            qvsmsg::DEVICE_GROUP_CHANGED.with(&[&d.location, &changed]),
        ));
    }
    for other in &d.with {
        if !all.iter().any(|o| o.location == *other && o.released) {
            return Some((
                Blocker::GroupNotReleased,
                qvsmsg::DEVICE_GROUP_SPLIT.with(&[&d.location, other]),
            ));
        }
    }
    if let Some(why) = d.releasable.first() {
        return Some((blocker_of(why), why.clone()));
    }
    if d.fenced {
        return Some((Blocker::Fenced, qvsmsg::DEVICE_FENCED.with(&[&d.location])));
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::released::ReleasedDevice;

    pub(crate) fn pci(address: &str, location: &str, class: &str, group: u32) -> Pci {
        Pci {
            address: address.into(),
            vendor: "8086".into(),
            device: "10d3".into(),
            class: class.into(),
            driver: None,
            location: location.into(),
            group: Some(group),
            reset: vec!["flr".into()],
            interrupts: vec!["msix".into(), "intx".into()],
            boot_display: false,
        }
    }

    /// A host like the harness's: a load-source virtio disk, a
    /// management adapter, a pool disk's controller, a network adapter
    /// and a disk controller each alone, a pair that share a group, a
    /// bridge in the pair's group, a display adapter.
    pub(crate) fn host() -> Vec<Pci> {
        vec![
            pci("0000:00:01.0", "P0000-B00-D01-F0", "030000", 1),
            pci("0000:00:02.0", "P0000-B00-D02-F0", "010000", 2),
            pci("0000:00:03.0", "P0000-B00-D03-F0", "020000", 3),
            pci("0000:00:04.0", "P0000-B00-D04-F0", "010000", 4),
            pci("0000:00:05.0", "P0000-B00-D05-F0", "020000", 5),
            pci("0000:00:06.0", "P0000-B00-D06-F0", "010802", 6),
            pci("0000:00:07.0", "P0000-B00-D07-F0", "020000", 7),
            pci("0000:00:07.1", "P0000-B00-D07-F1", "020000", 7),
            pci("0000:00:1e.0", "P0000-B00-D1E-F0", "060400", 7),
        ]
    }

    pub(crate) fn uses() -> HostUses {
        HostUses {
            load_source: vec!["P0000-B00-D02-F0".into()],
            pool_members: vec![("POOL1".into(), "P0000-B00-D04-F0".into())],
            management: vec!["P0000-B00-D03-F0-E0".into()],
            uplinks: Vec::new(),
        }
    }

    pub(crate) const ON: Iommu = Iommu {
        translation: true,
        remapping: true,
    };

    pub(crate) fn devices(pci: &[Pci], iommu: &Iommu, released: &Released) -> Vec<Device> {
        assess(&Facts {
            pci,
            iommu,
            uses: &uses(),
            released,
            holders: &[],
            fenced: &[],
        })
    }

    pub(crate) fn at<'d>(devices: &'d [Device], location: &str) -> &'d Device {
        devices.iter().find(|d| d.location == location).unwrap()
    }

    #[test]
    fn the_host_s_devices_cannot_be_released_and_say_why() {
        let d = devices(&host(), &ON, &Released::default());
        let why = |l: &str| at(&d, l).releasable.first().map(|i| i.text.clone());
        assert!(why("P0000-B00-D01-F0").unwrap().contains("display adapter"));
        assert!(why("P0000-B00-D02-F0").unwrap().contains("load-source"));
        assert!(why("P0000-B00-D03-F0").unwrap().contains("management"));
        assert!(why("P0000-B00-D04-F0").unwrap().contains("POOL1"));
        assert!(why("P0000-B00-D1E-F0").unwrap().contains("bridge"));
        for free in ["P0000-B00-D05-F0", "P0000-B00-D06-F0", "P0000-B00-D07-F0"] {
            assert_eq!(why(free), None, "{free}");
            let b = &at(&d, free).blocked;
            assert_eq!(b.as_ref().map(|(b, _)| *b), Some(Blocker::NotReleased));
        }
        assert_eq!(
            at(&d, "P0000-B00-D02-F0").blocked.as_ref().map(|(b, _)| *b),
            Some(Blocker::HostUses)
        );
    }

    #[test]
    fn a_group_goes_whole_without_its_bridge() {
        let d = devices(&host(), &ON, &Released::default());
        assert_eq!(at(&d, "P0000-B00-D07-F0").with, ["P0000-B00-D07-F1"]);
        assert_eq!(at(&d, "P0000-B00-D07-F1").with, ["P0000-B00-D07-F0"]);
        assert!(at(&d, "P0000-B00-D05-F0").with.is_empty());
    }

    #[test]
    fn a_used_device_keeps_its_whole_group() {
        let mut pci = host();
        // The management NIC moves into the NVMe controller's group.
        pci[2].group = Some(6);
        let d = devices(&pci, &ON, &Released::default());
        let nvme = at(&d, "P0000-B00-D06-F0");
        assert!(
            nvme.releasable[0].is(&qvsmsg::DEVICE_GROUP_HOST),
            "{nvme:?}"
        );
        assert!(nvme.releasable[0].text.contains("P0000-B00-D03-F0"));
    }

    #[test]
    fn every_safety_rule_blocks_on_its_own() {
        let mut pci = host();
        pci[4].reset.clear();
        pci[5].interrupts = vec!["intx".into()];
        pci[7].reset.clear();
        let d = devices(&pci, &ON, &Released::default());
        assert!(at(&d, "P0000-B00-D05-F0").releasable[0].is(&qvsmsg::DEVICE_NO_RESET));
        assert!(at(&d, "P0000-B00-D06-F0").releasable[0].is(&qvsmsg::DEVICE_NO_MSI));
        // One function of the pair cannot reset: neither can go.
        let f0 = &at(&d, "P0000-B00-D07-F0").releasable;
        assert!(
            f0.iter()
                .any(|i| i.is(&qvsmsg::DEVICE_NO_RESET) && i.text.contains("D07-F1"))
        );
        let off = Iommu {
            translation: true,
            remapping: false,
        };
        let d = devices(&host(), &off, &Released::default());
        let nic = at(&d, "P0000-B00-D05-F0");
        assert!(nic.releasable[0].is(&qvsmsg::NO_REMAPPING));
        assert_eq!(nic.blocked.as_ref().unwrap().0, Blocker::NoRemapping);
        let none = Iommu::default();
        let d = devices(&host(), &none, &Released::default());
        assert!(at(&d, "P0000-B00-D06-F0").releasable[0].is(&qvsmsg::NO_IOMMU));
    }

    #[test]
    fn released_devices_are_assignable_until_fenced_held_or_gone() {
        let pci = host();
        let mut released = Released::default();
        released.devices.push(ReleasedDevice::of(&pci[5], &[]));
        let d = devices(&pci, &ON, &released);
        assert!(at(&d, "P0000-B00-D06-F0").assignable());
        let fenced = vec!["P0000-B00-D06-F0".to_string()];
        let holders = vec![(
            "P0000-B00-D06-F0".to_string(),
            Holder {
                partition: 3,
                name: "PROD".into(),
                profile: "NORMAL".into(),
            },
        )];
        let d = assess(&Facts {
            pci: &pci,
            iommu: &ON,
            uses: &uses(),
            released: &released,
            holders: &holders,
            fenced: &fenced,
        });
        let nvme = at(&d, "P0000-B00-D06-F0");
        assert_eq!(nvme.blocked.as_ref().unwrap().0, Blocker::Fenced);
        assert_eq!(nvme.holder.as_ref().unwrap().name, "PROD");
        assert!(!nvme.assignable());
        // Gone from the machine: listed, released, blocked.
        let d = devices(&pci[..5], &ON, &released);
        let gone = at(&d, "P0000-B00-D06-F0");
        assert!(!gone.present && gone.released);
        assert_eq!(gone.blocked.as_ref().unwrap().0, Blocker::Gone);
    }

    #[test]
    fn a_disk_or_interface_is_on_the_longest_matching_code() {
        let mut pci = host();
        pci.push(pci_at("SPCIE-SLOT-3"));
        pci.push(pci_at("SPCIE-SLOT-3-F1"));
        let on = |code: &str| controller(code, &pci).map(|p| p.location.as_str());
        assert_eq!(on("SPCIE-SLOT-3-F1-T0.0.0.0"), Some("SPCIE-SLOT-3-F1"));
        assert_eq!(on("SPCIE-SLOT-3-N1"), Some("SPCIE-SLOT-3"));
        assert_eq!(on("SPCIE-SLOT-3-E0"), Some("SPCIE-SLOT-3"));
        assert_eq!(on("P0000-B00-D02-F0"), Some("P0000-B00-D02-F0"));
        assert_eq!(on("SPCIE-SLOT-33-E0"), None);
        assert_eq!(on("NOPCI-MMCBLK0"), None);
    }

    fn pci_at(location: &str) -> Pci {
        Pci {
            location: location.into(),
            address: format!("x{location}"),
            ..Pci::default()
        }
    }

    #[test]
    fn kinds_are_named_by_class() {
        assert_eq!(kind("010802"), "NVMe controller");
        assert_eq!(kind("020000"), "Ethernet adapter");
        assert_eq!(kind("0c0400"), "Fibre Channel adapter");
        assert_eq!(kind("ff0000"), "PCI device");
        assert!(is_bridge("060400"));
        assert!(!is_bridge("020000"));
    }
}
