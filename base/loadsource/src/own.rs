// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Which disks are the host's own. A disk with the appliance's three
//! partitions is not the host's for that alone: a partition's disk can
//! look the same, and so can a disk moved from another host. The host
//! takes the disk it started from and the members the label on that disk
//! names, and no other.
//!
//! The disk of the IPL is the one the firmware started: `BootCurrent`
//! names a boot entry, and the entry's device path has the PCI
//! functions down to the disk's controller, the GUID of the ESP, or
//! both. With no firmware to ask, or an entry that fits no disk, it is
//! the first disk in name order that has the three partitions.

use std::fs;
use std::path::Path;

use diskimg::efi;
use uuid::Uuid;

use crate::set::SetLabel;

/// What a boot entry's device path says of the disk it starts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Booted {
    /// The PCI functions from the root bridge down, each a device and
    /// a function number. Empty in a short-form path.
    pub pci: Vec<(u8, u8)>,
    /// The GUID of the partition the entry's file is on.
    pub partition: Option<Uuid>,
}

/// The most device path nodes read from one entry.
const MAX_NODES: usize = 32;

impl Booted {
    /// From a load option (`EFI_LOAD_OPTION`): attributes, the length
    /// of the device path list, the description, the list.
    pub fn of_load_option(option: &[u8]) -> Option<Self> {
        let list_len = usize::from(u16::from_le_bytes(option.get(4..6)?.try_into().ok()?));
        // The description: UTF-16, ended by a zero.
        let mut at = 6;
        loop {
            let unit = option.get(at..at + 2)?;
            at += 2;
            if unit == [0, 0] {
                break;
            }
        }
        let list = option.get(at..at.checked_add(list_len)?)?;
        Some(Self::of_device_path(list))
    }

    /// From a device path: its first instance, as far as it reads.
    pub fn of_device_path(mut path: &[u8]) -> Self {
        let mut booted = Self::default();
        for _ in 0..MAX_NODES {
            let Some(head) = path.get(..4) else { break };
            let (kind, sub) = (head[0], head[1]);
            let len = usize::from(u16::from_le_bytes([head[2], head[3]]));
            let Some(node) = path.get(4..len).filter(|_| len >= 4) else {
                break;
            };
            match (kind, sub) {
                // The end of the path, or of its first instance.
                (0x7F, _) => break,
                // Hardware, PCI: the function, then the device.
                (1, 1) if node.len() >= 2 => booted.pci.push((node[1], node[0])),
                // Media, hard drive: a GPT partition's GUID at 20,
                // when the signature type at 37 says GUID.
                (4, 1) if node.len() >= 38 && node[37] == 2 => {
                    booted.partition = uuid_le(&node[20..36]);
                }
                _ => {}
            }
            path = &path[len..];
        }
        booted
    }

    /// The entry `BootCurrent` names, under `vars` (efivarfs). `None`
    /// when the host did not come up through firmware that says.
    pub fn current(vars: &Path) -> Option<Self> {
        let current = fs::read(vars.join(efi::var_name("BootCurrent"))).ok()?;
        let number = u16::from_le_bytes(current.get(4..6)?.try_into().ok()?);
        let entry = fs::read(vars.join(efi::var_name(&efi::boot_name(number)))).ok()?;
        // The variable's attributes come before the load option.
        Self::of_load_option(entry.get(4..)?)
    }
}

fn uuid_le(bytes: &[u8]) -> Option<Uuid> {
    Some(Uuid::from_bytes_le(bytes.try_into().ok()?))
}

/// The PCI functions in a device's sysfs path, from the root bridge
/// down: each component of the form `ssss:bb:dd.f`.
pub fn pci_chain(sysfs_path: &Path) -> Vec<(u8, u8)> {
    sysfs_path
        .components()
        .filter_map(|c| {
            let name = c.as_os_str().to_str()?;
            let mut parts = name.split(':');
            let (segment, bus, rest) = (parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() || segment.len() != 4 || bus.len() != 2 {
                return None;
            }
            let (device, function) = rest.split_once('.')?;
            Some((
                u8::from_str_radix(device, 16).ok()?,
                u8::from_str_radix(function, 16).ok()?,
            ))
        })
        .collect()
}

/// A disk that has the appliance's three partitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The PCI functions down to its controller.
    pub pci: Vec<(u8, u8)>,
    /// The GUID of its ESP.
    pub esp: Uuid,
    /// The GUID of its data partition: what a label names a member by.
    pub data: Uuid,
    /// The newest label on the disk itself, when it has one.
    pub label: Option<SetLabel>,
}

/// The place in `candidates`, which are in name order, of the disk the
/// host started from.
pub fn ipl_disk(candidates: &[Candidate], booted: Option<&Booted>) -> Option<usize> {
    let mut left: Vec<usize> = (0..candidates.len()).collect();
    if let Some(booted) = booted {
        // Each thing the entry says narrows the choice when a disk
        // fits it, and is passed over when none does.
        let mut narrow = |fits: &dyn Fn(&Candidate) -> bool| {
            let fitting: Vec<usize> = left
                .iter()
                .copied()
                .filter(|i| fits(&candidates[*i]))
                .collect();
            if !fitting.is_empty() {
                left = fitting;
            }
        };
        if !booted.pci.is_empty() {
            narrow(&|c| c.pci == booted.pci);
        }
        if let Some(partition) = booted.partition {
            narrow(&|c| c.esp == partition);
        }
    }
    left.first().copied()
}

/// The places in `candidates` of the host's own disks: the disk of the
/// IPL, and the others the label on that disk names as members of its
/// set. A named disk that holds a label of another set is not taken.
pub fn own(candidates: &[Candidate], booted: Option<&Booted>) -> Vec<usize> {
    let Some(first) = ipl_disk(candidates, booted) else {
        return Vec::new();
    };
    let label = candidates[first].label.as_ref();
    (0..candidates.len())
        .filter(|i| {
            *i == first
                || label.is_some_and(|label| {
                    let other = &candidates[*i];
                    label.state_of(other.data).is_some()
                        && other.label.as_ref().is_none_or(|l| l.id == label.id)
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::{Member, MemberState};

    fn guid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn disk(pci: (u8, u8), esp: u128, data: u128) -> Candidate {
        Candidate {
            pci: vec![pci],
            esp: guid(esp),
            data: guid(data),
            label: None,
        }
    }

    fn label(id: u128, members: &[u128]) -> SetLabel {
        SetLabel {
            id: guid(id),
            blocks: 1,
            clean: true,
            written: 0,
            members: members
                .iter()
                .map(|m| Member {
                    partition: guid(*m),
                    state: MemberState::Active,
                    serial: String::new(),
                    location: String::new(),
                })
                .collect(),
        }
    }

    /// A load option whose path is `nodes`.
    fn option(nodes: &[(u8, u8, Vec<u8>)]) -> Vec<u8> {
        let mut list = Vec::new();
        for (kind, sub, body) in nodes {
            list.extend_from_slice(&[*kind, *sub]);
            list.extend_from_slice(&u16::try_from(body.len() + 4).unwrap().to_le_bytes());
            list.extend_from_slice(body);
        }
        list.extend_from_slice(&[0x7F, 0xFF, 4, 0]);
        let mut out = vec![1, 0, 0, 0];
        out.extend_from_slice(&u16::try_from(list.len()).unwrap().to_le_bytes());
        for unit in "QS/DEMO slot A".encode_utf16().chain([0]) {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&list);
        out
    }

    fn hd(partition: Uuid) -> Vec<u8> {
        let mut body = vec![0u8; 38];
        body[0] = 1;
        body[20..36].copy_from_slice(&partition.to_bytes_le());
        body[36] = 2;
        body[37] = 2;
        body
    }

    #[test]
    fn a_load_option_reads_as_its_pci_functions_and_its_partition() {
        let esp = guid(0xE5);
        let full = option(&[
            (2, 1, vec![0xD0, 0x41, 0x03, 0x0A, 0, 0, 0, 0]),
            (1, 1, vec![0, 0x1C]),
            (1, 1, vec![2, 4]),
            (4, 1, hd(esp)),
            (4, 4, b"\\\0E\0F\0I\0\0\0".to_vec()),
        ]);
        let booted = Booted::of_load_option(&full).unwrap();
        assert_eq!(booted.pci, [(0x1C, 0), (4, 2)]);
        assert_eq!(booted.partition, Some(esp));
        // A short-form path: the partition alone.
        let short = Booted::of_load_option(&option(&[(4, 1, hd(esp))])).unwrap();
        assert_eq!((short.pci.len(), short.partition), (0, Some(esp)));
        // What is cut short reads as far as it goes, and never panics.
        for cut in 0..full.len() {
            let _ = Booted::of_load_option(&full[..cut]);
        }
        assert_eq!(Booted::of_device_path(&[1, 1, 2, 0]), Booted::default());
    }

    #[test]
    fn a_sysfs_path_gives_its_pci_functions() {
        let virtio = Path::new("/sys/devices/pci0000:00/0000:00:04.0/virtio2/block/vda");
        assert_eq!(pci_chain(virtio), [(4, 0)]);
        let nvme =
            Path::new("/sys/devices/pci0000:00/0000:00:1c.4/0000:02:00.0/nvme/nvme0/nvme0n1");
        assert_eq!(pci_chain(nvme), [(0x1C, 4), (0, 0)]);
        assert!(pci_chain(Path::new("/sys/devices/virtual/block/ublkb0")).is_empty());
    }

    #[test]
    fn the_disk_of_the_ipl_is_the_one_the_entry_names() {
        // Two disks from one build: the same GUIDs on both.
        let disks = [disk((3, 0), 0xE5, 0xDA), disk((4, 0), 0xE5, 0xDA)];
        let by_pci = |pci| Booted {
            pci: vec![pci],
            partition: None,
        };
        assert_eq!(ipl_disk(&disks, Some(&by_pci((4, 0)))), Some(1));
        assert_eq!(ipl_disk(&disks, Some(&by_pci((3, 0)))), Some(0));
        // An entry that fits no disk, and no firmware: the first.
        assert_eq!(ipl_disk(&disks, Some(&by_pci((9, 0)))), Some(0));
        assert_eq!(ipl_disk(&disks, None), Some(0));
        assert_eq!(ipl_disk(&[], None), None);
        // A short-form entry: by the ESP's GUID.
        let disks = [disk((3, 0), 0xA, 0x1), disk((4, 0), 0xB, 0x2)];
        let by_esp = Booted {
            pci: Vec::new(),
            partition: Some(guid(0xB)),
        };
        assert_eq!(ipl_disk(&disks, Some(&by_esp)), Some(1));
    }

    #[test]
    fn a_disk_that_only_looks_like_a_load_source_is_left_alone() {
        // The host's disk, and a guest's that has the same partitions.
        let disks = [disk((3, 0), 0xA, 0x1), disk((4, 0), 0xB, 0x2)];
        assert_eq!(own(&disks, None), [0]);
        let second = Booted {
            pci: vec![(4, 0)],
            partition: None,
        };
        assert_eq!(own(&disks, Some(&second)), [1]);
    }

    #[test]
    fn the_members_the_label_names_are_taken_and_no_other() {
        let mut disks = vec![
            disk((3, 0), 0xA, 0x1),
            disk((4, 0), 0xB, 0x2),
            disk((5, 0), 0xC, 0x3),
        ];
        disks[0].label = Some(label(7, &[0x1, 0x2]));
        disks[1].label = Some(label(7, &[0x1, 0x2]));
        assert_eq!(own(&disks, None), [0, 1]);
        // Started from the second disk, the first still its member.
        let second = Booted {
            pci: vec![(4, 0)],
            partition: None,
        };
        assert_eq!(own(&disks, Some(&second)), [0, 1]);
        // A member that is blank yet, being prepared: named, no label.
        disks[1].label = None;
        assert_eq!(own(&disks, None), [0, 1]);
        // A disk another host's set names by the same partition.
        disks[1].label = Some(label(8, &[0x1, 0x2]));
        assert_eq!(own(&disks, None), [0]);
    }
}
