// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The adapters under load: each Ethernet link brought up, and on its
//! IPv6 link-local address, datagrams sent to the all-nodes group as
//! fast as the link takes them; the datagrams of other soak partitions
//! on the same switch are checked as they arrive. Every one carries its
//! sender, its sequence and a fill of both, so a byte changed between
//! the two partitions' memories is seen.

use std::net::{Ipv6Addr, SocketAddrV6, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{Counts, bad, word};

/// The port the soak partitions use.
const PORT: u16 = 47_230;
/// A datagram's bytes: a full frame's worth.
const BYTES: usize = 1400;
const ALL_NODES: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1);

/// The Ethernet links: name and index, the loopback left out.
fn links() -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = std::fs::read_to_string(entry.path().join("type")).unwrap_or_default();
        let index = std::fs::read_to_string(entry.path().join("ifindex"))
            .ok()
            .and_then(|i| i.trim().parse::<u32>().ok());
        if kind.trim() == "1"
            && name != "lo"
            && let Some(index) = index
        {
            out.push((name, index));
        }
    }
    out
}

/// A datagram: `sender`, `sequence`, then a fill seeded with both.
fn datagram(sender: u64, sequence: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(BYTES);
    out.extend_from_slice(&sender.to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    let seed = sender ^ (sequence << 17);
    let mut i = 0u64;
    while out.len() < BYTES {
        out.extend_from_slice(&word(seed, i).to_le_bytes());
        i += 1;
    }
    out.truncate(BYTES);
    out
}

/// Whether a datagram received is whole: its sender and sequence, or
/// where it differs.
fn whole(bytes: &[u8]) -> Result<(u64, u64), String> {
    if bytes.len() != BYTES {
        return Err(format!("{} bytes", bytes.len()));
    }
    let sender = u64::from_le_bytes(bytes[0..8].try_into().unwrap_or([0; 8]));
    let sequence = u64::from_le_bytes(bytes[8..16].try_into().unwrap_or([0; 8]));
    let wanted = datagram(sender, sequence);
    match bytes.iter().zip(&wanted).position(|(a, b)| a != b) {
        None => Ok((sender, sequence)),
        Some(at) => Err(format!(
            "byte {at} of sender {sender:x} sequence {sequence}: wanted {:02x} found {:02x}",
            wanted[at], bytes[at]
        )),
    }
}

/// Bring every link up and start a sender and a receiver on each.
pub fn start(counts: &Arc<Counts>) {
    let found = links();
    if let Ok(mut rtnl) = netlink::route::Rtnl::open() {
        for (_, index) in &found {
            let _ = rtnl.set_up(*index, true);
        }
    }
    let me = u64::from(std::process::id()) ^ (u64::from(rand_seed()) << 32);
    for (name, index) in found {
        let c = counts.clone();
        std::thread::spawn(move || link(&c, &name, index, me));
    }
}

/// Something of this IPL to tell senders apart: the boot's id.
fn rand_seed() -> u32 {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .and_then(|s| u32::from_str_radix(&s.trim().replace('-', "")[..8], 16).ok())
        .unwrap_or(0)
}

fn link(counts: &Arc<Counts>, name: &str, index: u32, me: u64) {
    // The link-local address needs a moment after the link is up.
    let socket = loop {
        match UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, PORT, 0, index)) {
            Ok(s) => break s,
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    };
    let _ = socket.join_multicast_v6(&ALL_NODES, index);
    let _ = socket.set_multicast_loop_v6(false);
    let Ok(receiver) = socket.try_clone() else {
        return bad(counts, &format!("net {name}: no second handle"));
    };
    let c = counts.clone();
    let name_rx = name.to_string();
    std::thread::spawn(move || {
        let mut buffer = vec![0u8; 2048];
        loop {
            match receiver.recv_from(&mut buffer) {
                Ok((n, _)) => match whole(&buffer[..n]) {
                    Ok((sender, _)) if sender != me => {
                        c.rx.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(_) => {}
                    Err(e) => bad(&c, &format!("net {name_rx} received: {e}")),
                },
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    });
    let to = SocketAddrV6::new(ALL_NODES, PORT, 0, index);
    let mut sequence = 0u64;
    loop {
        match socket.send_to(&datagram(me, sequence), to) {
            Ok(_) => {
                counts.tx.fetch_add(1, Ordering::Relaxed);
                sequence += 1;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
        // A steady stream, not a flood that starves the others.
        if sequence.is_multiple_of(64) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_datagram_is_whole_until_a_byte_changes() {
        let mut d = datagram(0xABC, 77);
        assert_eq!(whole(&d), Ok((0xABC, 77)));
        d[600] ^= 1;
        assert!(whole(&d).unwrap_err().contains("byte 600"));
        assert!(whole(&d[..100]).is_err());
    }
}
