// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a disk of the host is, by what is on it: the load source, a
//! member or a hot spare of one of this host's pools, a pool disk of
//! another host, a disk with someone else's data, or nothing. The host
//! takes a disk only when its label is this host's; anything else is
//! shown for what it is and is reused only with a typed confirmation.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use loadsource::gpt;
use loadsource::multi::{At, Copies};
use uuid::Uuid;

use crate::label::{self, PoolLabel};
use crate::layout::{LABEL_OFFSET, member_type, record_type};
use crate::names::PoolDiskNames;

/// Bytes read from a disk's start to tell what is on it.
pub const HEAD_BYTES: usize = 40 * 1024;

/// What a disk is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The host's load source.
    LoadSource,
    /// A unit of one of this host's pools: the pool's newest label as
    /// this disk has it, and the disk's member partition.
    Pool {
        /// The label on the disk.
        label: PoolLabel,
        /// Its member partition's GUID.
        partition: Uuid,
    },
    /// A pool disk of another host.
    ForeignPool {
        /// The pool's name on that host.
        name: String,
    },
    /// Data that is not the host's: a partition table, a file system,
    /// another volume manager's label.
    Foreign(&'static str),
    /// Nothing the host can see.
    NotConfigured,
}

impl Kind {
    /// As Display disk configuration words it.
    pub fn word(&self) -> String {
        match self {
            Kind::LoadSource => "Load source".into(),
            Kind::Pool { label, partition } => match label.unit_of(*partition) {
                Some((_, u)) if u.state == label::UnitState::Spare => {
                    format!("Hot spare, {}", label.name)
                }
                Some((n, u)) => format!("{} unit {n}, {}", label.name, u.state.word()),
                None => format!("{}, not a unit", label.name),
            },
            Kind::ForeignPool { name } => format!("Foreign pool {name}"),
            Kind::Foreign(what) => format!("Foreign: {what}"),
            Kind::NotConfigured => "Not configured".into(),
        }
    }

    /// Whether a pool may be made of the disk without a confirmation.
    pub fn free(&self) -> bool {
        matches!(self, Kind::NotConfigured)
    }
}

/// What the first bytes of a disk say is on it, when it is not a pool
/// disk: `None` for nothing recognized.
pub fn signature(head: &[u8]) -> Option<&'static str> {
    let at = |i: usize, bytes: &[u8]| head.get(i..i + bytes.len()) == Some(bytes);
    if at(512, b"EFI PART") || at(4096, b"EFI PART") {
        Some("a GPT partition table")
    } else if at(0, b"XFSB") {
        Some("an xfs file system")
    } else if at(1080, &[0x53, 0xEF]) {
        Some("an ext file system")
    } else if at(512, b"LABELONE") {
        Some("an LVM volume")
    } else if at(4096, &[0xFC, 0x4E, 0x2B, 0xA9]) {
        Some("a software RAID member")
    } else if at(32769, b"CD001") {
        Some("an ISO 9660 image")
    } else if at(3, b"NTFS    ") {
        Some("an NTFS file system")
    } else if at(510, &[0x55, 0xAA]) {
        Some("an MBR partition table")
    } else {
        None
    }
}

/// The disk named `node`, as this host (`host`) sees it.
pub fn classify(names: &PoolDiskNames, node: &Path, host: [u8; 16]) -> Kind {
    let Ok(mut file) = File::open(node) else {
        return Kind::NotConfigured;
    };
    let mut head = vec![0u8; HEAD_BYTES];
    let read = file
        .seek(SeekFrom::Start(0))
        .and_then(|_| file.read(&mut head));
    let Ok(n) = read else {
        return Kind::NotConfigured;
    };
    head.truncate(n);
    let table = gpt::entries(&mut file).unwrap_or_default();
    let record = table.iter().find(|e| e.type_guid == record_type(names));
    let member = table.iter().find(|e| e.type_guid == member_type(names));
    if let (Some(record), Some(member)) = (record, member) {
        return match read_label(names, &mut file, record.offset()) {
            Some(l) if l.host == host => Kind::Pool {
                label: l,
                partition: member.guid,
            },
            Some(l) => Kind::ForeignPool { name: l.name },
            None => Kind::Foreign("a pool disk whose label cannot be read"),
        };
    }
    signature(&head).map_or(Kind::NotConfigured, Kind::Foreign)
}

/// The newest label in a record partition at `offset` of `dev`.
pub fn read_label<D: Read + Seek>(names: &PoolDiskNames, dev: D, offset: u64) -> Option<PoolLabel> {
    let mut copies = Copies::new(
        label::spec(names),
        vec![At::new(ReadOnly(dev), offset + LABEL_OFFSET)],
    );
    PoolLabel::decode(&copies.read().ok()?.payload)
}

/// A device read through `abrec`, which wants to be able to write.
struct ReadOnly<D>(D);

impl<D: Read> Read for ReadOnly<D> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl<D> std::io::Write for ReadOnly<D> {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("read only"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<D: Seek> Seek for ReadOnly<D> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

impl<D: Read + Seek> abrec::Device for ReadOnly<D> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_name_what_is_on_a_disk() {
        let mut head = vec![0u8; HEAD_BYTES];
        assert_eq!(signature(&head), None);
        head[510] = 0x55;
        head[511] = 0xAA;
        assert_eq!(signature(&head), Some("an MBR partition table"));
        head[512..520].copy_from_slice(b"EFI PART");
        assert_eq!(signature(&head), Some("a GPT partition table"));
        let mut xfs = vec![0u8; HEAD_BYTES];
        xfs[..4].copy_from_slice(b"XFSB");
        assert_eq!(signature(&xfs), Some("an xfs file system"));
        let mut iso = vec![0u8; HEAD_BYTES];
        iso[32769..32774].copy_from_slice(b"CD001");
        assert_eq!(signature(&iso), Some("an ISO 9660 image"));
        // Short heads are read as far as they go.
        assert_eq!(signature(&[0u8; 10]), None);
    }

    #[test]
    fn only_zeros_are_free() {
        assert!(Kind::NotConfigured.free());
        assert!(!Kind::Foreign("x").free());
        assert!(!Kind::ForeignPool { name: "P".into() }.free());
        assert_eq!(
            Kind::Foreign("an ext file system").word(),
            "Foreign: an ext file system"
        );
    }
}
