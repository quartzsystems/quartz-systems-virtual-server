// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The requests against a live kernel, with frames through taps. It
//! needs a network namespace of its own with the right to change it:
//!
//! ```text
//! NL_LIVE=1 unshare -rn cargo test -p nl --test live
//! ```
//!
//! Without `NL_LIVE` it does nothing.

#![cfg(target_os = "linux")]
#![allow(unsafe_code, clippy::missing_panics_doc)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::time::Duration;

use nl::{Nl, Origin, PortRules, PortVlan};

const HDR: usize = 12;

fn live() -> bool {
    std::env::var_os("NL_LIVE").is_some()
}

struct Tap {
    file: File,
    index: u32,
    mac: [u8; 6],
}

fn tap(nl: &mut Nl, name: &str, n: u8, bridge: u32, vlans: &[PortVlan], tagged: &[u16]) -> Tap {
    let file = nl::tap::create(name).unwrap();
    // SAFETY: setting O_NONBLOCK on a descriptor this owns.
    unsafe {
        let flags = libc::fcntl(file.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
    let link = nl.rtnl().link_by_name(name).unwrap();
    nl.disable_ipv6(name).unwrap();
    nl.set_master(link.index, bridge).unwrap();
    nl.port_rules(link.index, PortRules::PARTITION).unwrap();
    nl.port_vlans(link.index, vlans, true).unwrap();
    let mac = [0x02, 0x51, 0x56, 0, 0, n];
    for v in vlans {
        nl.fdb_static(link.index, mac, v.vid, true).unwrap();
    }
    nl.ingress_filter(link.index, tagged).unwrap();
    nl.rtnl().set_up(link.index, true).unwrap();
    Tap {
        file,
        index: link.index,
        mac,
    }
}

fn frame(dst: [u8; 6], src: [u8; 6], vlan: Option<u16>, mark: u8) -> Vec<u8> {
    let mut f = vec![0u8; HDR];
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    if let Some(v) = vlan {
        f.extend_from_slice(&[0x81, 0x00]);
        f.extend_from_slice(&v.to_be_bytes());
    }
    // A local experimental Ethernet type, and the mark.
    f.extend_from_slice(&[0x88, 0xB5, mark]);
    f.resize(HDR + 60, 0);
    f
}

/// The marks of the frames `t` has received since the last look.
fn received(t: &mut Tap) -> Vec<(Option<u16>, u8)> {
    std::thread::sleep(Duration::from_millis(150));
    let mut out = Vec::new();
    let mut buf = [0u8; 2048];
    while let Ok(n) = t.file.read(&mut buf) {
        let f = &buf[HDR..n];
        if f.len() < 17 {
            continue;
        }
        let (vlan, rest) = if f[12..14] == [0x81, 0x00] {
            (Some(u16::from_be_bytes([f[14], f[15]]) & 0x0FFF), &f[16..])
        } else {
            (None, &f[12..])
        };
        if rest[0..2] == [0x88, 0xB5] {
            out.push((vlan, rest[2]));
        }
    }
    out
}

#[test]
fn a_switchs_ports_pass_what_a_partition_may_send_and_nothing_else() {
    if !live() {
        return;
    }
    let mut nl = Nl::open().unwrap();
    let br = nl.create_bridge("tbr0", 1500).unwrap();
    nl.disable_ipv6("tbr0").unwrap();
    nl.rtnl().set_up(br.index, true).unwrap();
    let mut a = tap(&mut nl, "ta0", 1, br.index, &[PortVlan::native(10)], &[]);
    let mut b = tap(
        &mut nl,
        "tb0",
        2,
        br.index,
        &[PortVlan::native(10), PortVlan::tagged(30)],
        &[30],
    );
    let mut c = tap(
        &mut nl,
        "tc0",
        3,
        br.index,
        &[PortVlan::native(20), PortVlan::tagged(30)],
        &[30],
    );
    // The bridge itself is in no VLAN: nothing goes up to the host.
    let vlans = nl.vlans().unwrap();
    let own = vlans.iter().find(|v| v.index == br.index);
    assert!(own.is_none_or(|v| v.vlans.is_empty()), "{own:?}");
    assert!(nl.ipv6_disabled("ta0"));
    let _ = (received(&mut a), received(&mut b), received(&mut c));

    let bcast = [0xFF; 6];
    // Untagged from A's own address: B, on VLAN 10, has it untagged.
    a.file.write_all(&frame(bcast, a.mac, None, 1)).unwrap();
    assert_eq!(received(&mut b), [(None, 1)]);
    assert_eq!(received(&mut c), []);
    // From another address: dropped at the locked port.
    a.file
        .write_all(&frame(bcast, [0x02, 0, 0, 0, 0, 9], None, 2))
        .unwrap();
    assert_eq!(received(&mut b), []);
    // A tagged frame from an access port, even of its own VLAN: dropped.
    a.file.write_all(&frame(bcast, a.mac, Some(10), 3)).unwrap();
    a.file.write_all(&frame(bcast, a.mac, Some(30), 4)).unwrap();
    assert_eq!(received(&mut b), []);
    assert_eq!(received(&mut c), []);
    // A listed tagged VLAN: C has it tagged, A nothing.
    b.file.write_all(&frame(bcast, b.mac, Some(30), 5)).unwrap();
    assert_eq!(received(&mut c), [(Some(30), 5)]);
    assert_eq!(received(&mut a), []);
    // A VLAN not listed for B: dropped.
    b.file.write_all(&frame(bcast, b.mac, Some(20), 6)).unwrap();
    assert_eq!(received(&mut c), []);
    // A BPDU: dropped.
    a.file
        .write_all(&frame([0x01, 0x80, 0xC2, 0, 0, 0], a.mac, None, 7))
        .unwrap();
    assert_eq!(received(&mut b), []);
    // Unknown unicast is not flooded to partitions; known goes to its port.
    a.file
        .write_all(&frame([0x02, 0, 0, 0, 0, 7], a.mac, None, 8))
        .unwrap();
    assert_eq!(received(&mut b), []);
    a.file.write_all(&frame(b.mac, a.mac, None, 9)).unwrap();
    assert_eq!(received(&mut b), [(None, 9)]);

    let table = nl.fdb(br.index).unwrap();
    let statics: Vec<_> = table
        .iter()
        .filter(|e| e.origin == Origin::Static)
        .map(|e| (e.port, e.vlan))
        .collect();
    for want in [
        (a.index, 10),
        (b.index, 10),
        (b.index, 30),
        (c.index, 20),
        (c.index, 30),
    ] {
        assert!(statics.contains(&want), "{want:?} in {table:?}");
    }
    assert!(
        !table
            .iter()
            .any(|e| e.origin == Origin::Learned && e.port == a.index)
    );
    let stats = nl.rtnl().link_by_index(a.index).unwrap().stats.unwrap();
    // Seven frames from A, dropped or not, the tap took them all.
    assert_eq!(stats.rx_packets, 7, "{stats:?}");

    // A tap goes with its last open file, and its port with it.
    drop(a);
    std::thread::sleep(Duration::from_millis(100));
    assert!(nl.rtnl().link_by_name("ta0").is_err());
}
