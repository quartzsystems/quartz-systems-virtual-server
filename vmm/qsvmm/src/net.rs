// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition's Ethernet adapters, opened as root: each port's tap,
//! the file the partition manager gave as a descriptor and taken as it
//! is, and `/dev/vhost-net` for it. An adapter without a tap came up
//! without its switch: its link is down. `qsvmm` opens no network
//! device by name.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

use devices::virtio::net::{Backend, Identity, Net};

use crate::config::Partition;

/// The device vhost-net is reached through.
pub const VHOST_NET: &str = "/dev/vhost-net";

/// The descriptor a `/proc/self/fd/N` path names.
pub fn descriptor(path: &Path) -> Option<i32> {
    path.strip_prefix("/proc/self/fd")
        .ok()?
        .to_str()?
        .parse()
        .ok()
}

/// A guest's address, as the report says it.
fn text(mac: [u8; 6]) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Every adapter of `partition`, by its slot.
pub fn open(partition: &Partition) -> io::Result<Vec<(u32, Net)>> {
    let mut out = Vec::with_capacity(partition.net.len());
    for n in &partition.net {
        let mac = n
            .mac_bytes()
            .ok_or_else(|| io::Error::other(format!("slot {}: the address", n.slot)))?;
        let identity = Identity {
            mac,
            mtu: u16::try_from(n.mtu).unwrap_or(u16::MAX),
        };
        let backend = match &n.tap {
            Some(path) => {
                let fd = descriptor(path).ok_or_else(|| {
                    io::Error::other(format!(
                        "slot {}: {} is no descriptor",
                        n.slot,
                        path.display()
                    ))
                })?;
                let tap: File = crate::sys::inherited(fd)
                    .map_err(|e| io::Error::other(format!("slot {}: its tap: {e}", n.slot)))?;
                let vhost = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(VHOST_NET)
                    .map_err(|e| io::Error::other(format!("slot {}: {VHOST_NET}: {e}", n.slot)))?;
                Some(Backend { tap, vhost })
            }
            None => None,
        };
        let slot = n.slot;
        // What the guest does with its address is said, and goes no
        // further: the port passes only the profile's.
        let said = Box::new(move |mac: [u8; 6]| {
            eprintln!("NET {slot} the guest set its address to {}", text(mac));
        });
        out.push((n.slot, Net::new(identity, backend, said)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_given_file_is_named_by_its_descriptor() {
        assert_eq!(descriptor(Path::new("/proc/self/fd/7")), Some(7));
        assert_eq!(descriptor(Path::new("/proc/self/fd/x")), None);
        assert_eq!(descriptor(Path::new("/dev/net/tun")), None);
        assert_eq!(text([2, 0x5A, 0, 0, 0, 9]), "02:5a:00:00:00:09");
    }
}
