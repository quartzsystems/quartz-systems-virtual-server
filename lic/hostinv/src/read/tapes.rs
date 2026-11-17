// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host's physical tape drives, from `/sys/class/scsi_tape`: each
//! drive's location code (a disk's, with its SCSI address), vendor,
//! product and serial number, and its SCSI generic device, which is how
//! a drive passed through is reached.

use std::path::Path;

use super::disks::{scsi_unit, serial};
use super::place::chain;
use crate::location::{Slot, unit_code};
use crate::sysfs::{names, numbered, resolve, text};

/// A physical tape drive.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TapeDrive {
    /// Its location code.
    pub location: String,
    /// Its vendor, as the drive says it.
    pub vendor: String,
    /// Its product.
    pub product: String,
    /// Its serial number.
    pub serial: String,
    /// Its SCSI generic device's name: `sg2`.
    pub sg: String,
    /// Its tape device's name: `st0`.
    pub st: String,
}

/// The tape drives, by their tape device's number.
pub(crate) fn read(sys: &Path, slots: &[Slot]) -> Vec<TapeDrive> {
    let class = sys.join("class/scsi_tape");
    let mut found: Vec<(u32, TapeDrive)> = names(&class)
        .into_iter()
        .filter_map(|name| {
            let number = numbered(&name, "st")?;
            let dir = class.join(&name);
            let node = resolve(sys, &dir.join("device"));
            let location = unit_code(&chain(&node.parts), slots, &name, scsi_unit(&node));
            let sg = names(dir.join("device/scsi_generic"))
                .into_iter()
                .next()
                .unwrap_or_default();
            Some((
                number,
                TapeDrive {
                    location,
                    vendor: text(dir.join("device/vendor")).unwrap_or_default(),
                    product: text(dir.join("device/model")).unwrap_or_default(),
                    serial: serial(&dir),
                    sg,
                    st: name,
                },
            ))
        })
        .collect();
    found.sort_by_key(|(n, _)| *n);
    found.into_iter().map(|(_, d)| d).collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fixture::Tree;

    #[test]
    fn a_drive_behind_an_adapter_is_read_with_its_generic_device() {
        let tree = Tree::new("tapes");
        let dev = "sys/devices/pci0000:00/0000:00:08.0/host2/target2:0:3/2:0:3:0";
        tree.file(&format!("{dev}/vendor"), "QUARTZ  \n");
        tree.file(&format!("{dev}/model"), "LTO8 DRIVE      \n");
        tree.file(&format!("{dev}/type"), "1\n");
        let mut page = vec![0u8, 0x80, 0, 10];
        page.extend_from_slice(b"1013000123");
        tree.file(&format!("{dev}/vpd_pg80"), page);
        tree.dir(&format!("{dev}/scsi_generic/sg2"));
        tree.dir(&format!("{dev}/scsi_tape/st0"));
        tree.link("sys/class/scsi_tape/st0", &format!("{dev}/scsi_tape/st0"));
        tree.link(&format!("{dev}/scsi_tape/st0/device"), dev);
        tree.dir("sys/class/scsi_tape/nst0");
        let found = read(&tree.sys(), &[]);
        assert_eq!(
            found,
            [TapeDrive {
                location: "P0000-B00-D08-F0-T0.0.3.0".into(),
                vendor: "QUARTZ".into(),
                product: "LTO8 DRIVE".into(),
                serial: "1013000123".into(),
                sg: "sg2".into(),
                st: "st0".into(),
            }]
        );
    }
}
