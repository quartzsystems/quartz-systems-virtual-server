// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The harness's own network, on the far side of the host's uplink
//! candidates. Each is a virtio NIC of the host on QEMU's `dgram`
//! backend, whose other end is a UDP socket of the harness: every frame
//! the host sends out of it arrives here whole, tags included, and what
//! the harness writes there arrives at the NIC. The NICs are one
//! segment, as the ports of one physical switch: the station answers out
//! of the NIC a frame came in on.
//!
//! The station keeps every frame it saw, answers ARP and ICMP echo for
//! its own address on each VLAN, and is a DHCP server: a client on
//! VLAN `v` is given `10.<v / 256>.<v % 256>.100` and on, the station
//! being `.1`; untagged, `10.255.0.0/24`.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The station's own MAC address.
pub const MAC: [u8; 6] = [0x02, 0x48, 0x41, 0x52, 0x4E, 0x01];
/// The most frames kept.
const KEPT: usize = 200_000;

/// The subnet of `vlan` (0 for untagged): its first three octets.
pub fn subnet(vlan: u16) -> [u8; 3] {
    if vlan == 0 {
        [10, 255, 0]
    } else {
        let [hi, lo] = vlan.to_be_bytes();
        [10, hi, lo]
    }
}

/// The station's address on `vlan`.
pub fn station_ip(vlan: u16) -> Ipv4Addr {
    let [a, b, c] = subnet(vlan);
    Ipv4Addr::new(a, b, c, 1)
}

/// One frame seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// Which NIC it came out of, from 0.
    pub nic: usize,
    /// Its outer VLAN; none when it was untagged.
    pub vlan: Option<u16>,
    /// Its destination.
    pub dst: [u8; 6],
    /// Its source.
    pub src: [u8; 6],
    /// Its Ethernet type, after the tag.
    pub ethertype: u16,
    /// Its length, whole.
    pub len: usize,
    /// When, since the station started.
    pub at: Duration,
}

/// One NIC of the station.
struct Nic {
    sock: UdpSocket,
    qemu: SocketAddr,
    /// Whether it is cut: what comes in is dropped, nothing goes out.
    cut: Arc<AtomicBool>,
}

#[derive(Default)]
struct State {
    seen: Vec<Seen>,
    /// Who has which address, by VLAN: as DHCP gave it or ARP said.
    neighbours: BTreeMap<(u16, Ipv4Addr), [u8; 6]>,
    /// The addresses given, by VLAN and client.
    leases: BTreeMap<(u16, [u8; 6]), Ipv4Addr>,
    /// The echo replies that came, by their identifier and sequence.
    replies: Vec<(u16, u16, Duration)>,
    /// Whether DHCP is answered.
    dhcp: bool,
}

/// The station.
pub struct Station {
    nics: Vec<Arc<Nic>>,
    state: Arc<Mutex<State>>,
    started: Instant,
    stop: Arc<AtomicBool>,
}

/// A NIC's two ports: the host side's (QEMU's) and the harness's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ports {
    /// QEMU's local port.
    pub qemu: u16,
    /// The harness's.
    pub harness: u16,
}

fn free_port() -> std::io::Result<u16> {
    Ok(UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn put_u16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_be_bytes());
}

/// The Internet checksum of `data`.
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let word = u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum > 0xFFFF {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !u16::try_from(sum).unwrap_or(0xFFFF)
}

/// An Ethernet frame: `dst`, `src`, a tag when `vlan` is some, `ethertype`
/// and `payload`.
pub fn frame(
    dst: [u8; 6],
    src: [u8; 6],
    vlan: Option<u16>,
    ethertype: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 18);
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    if let Some(v) = vlan {
        put_u16(&mut f, 0x8100);
        put_u16(&mut f, v & 0x0FFF);
    }
    put_u16(&mut f, ethertype);
    f.extend_from_slice(payload);
    if f.len() < 60 {
        f.resize(60, 0);
    }
    f
}

/// An IPv4 header and `body` of `protocol`.
fn ipv4(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, body: &[u8]) -> Vec<u8> {
    let mut h = vec![0x45, 0];
    put_u16(&mut h, u16::try_from(20 + body.len()).unwrap_or(u16::MAX));
    h.extend_from_slice(&[0, 0, 0x40, 0, 64, protocol, 0, 0]);
    h.extend_from_slice(&src.octets());
    h.extend_from_slice(&dst.octets());
    let sum = checksum(&h);
    h[10..12].copy_from_slice(&sum.to_be_bytes());
    h.extend_from_slice(body);
    h
}

/// A frame's outer tag and the rest after it.
fn untag(f: &[u8]) -> Option<(Option<u16>, u16, &[u8])> {
    let t = u16::from_be_bytes([*f.get(12)?, *f.get(13)?]);
    if t == 0x8100 || t == 0x88A8 {
        let vid = u16::from_be_bytes([*f.get(14)?, *f.get(15)?]) & 0x0FFF;
        let inner = u16::from_be_bytes([*f.get(16)?, *f.get(17)?]);
        Some((Some(vid), inner, f.get(18..)?))
    } else {
        Some((None, t, f.get(14..)?))
    }
}

impl Station {
    /// The ports of `count` NICs, to be given to QEMU.
    pub fn ports(count: usize) -> std::io::Result<Vec<Ports>> {
        (0..count)
            .map(|_| {
                Ok(Ports {
                    qemu: free_port()?,
                    harness: free_port()?,
                })
            })
            .collect()
    }

    /// The station on `ports`, answering DHCP when `dhcp`.
    pub fn start(ports: &[Ports], dhcp: bool) -> std::io::Result<Self> {
        let state = Arc::new(Mutex::new(State {
            dhcp,
            ..State::default()
        }));
        let started = Instant::now();
        let stop = Arc::new(AtomicBool::new(false));
        let mut nics = Vec::new();
        for p in ports {
            let sock = UdpSocket::bind(("127.0.0.1", p.harness))?;
            sock.set_read_timeout(Some(Duration::from_millis(100)))?;
            nics.push(Arc::new(Nic {
                sock,
                qemu: SocketAddr::from(([127, 0, 0, 1], p.qemu)),
                cut: Arc::new(AtomicBool::new(false)),
            }));
        }
        for (i, nic) in nics.iter().enumerate() {
            let nic = nic.clone();
            let state = state.clone();
            let stop = stop.clone();
            thread::Builder::new()
                .name(format!("wire{i}"))
                .spawn(move || serve(i, &nic, &state, started, &stop))?;
        }
        Ok(Self {
            nics,
            state,
            started,
            stop,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every frame seen so far.
    pub fn seen(&self) -> Vec<Seen> {
        self.lock().seen.clone()
    }

    /// Forget the frames seen.
    pub fn clear(&self) {
        let mut s = self.lock();
        s.seen.clear();
        s.replies.clear();
    }

    /// The address DHCP gave `mac` on `vlan`, when it did.
    pub fn lease(&self, vlan: u16, mac: [u8; 6]) -> Option<Ipv4Addr> {
        self.lock().leases.get(&(vlan, mac)).copied()
    }

    /// Every address given, with its VLAN and client.
    pub fn leases(&self) -> Vec<(u16, [u8; 6], Ipv4Addr)> {
        self.lock()
            .leases
            .iter()
            .map(|((v, m), ip)| (*v, *m, *ip))
            .collect()
    }

    /// Cut NIC `nic`, or join it again: a cable pulled.
    pub fn cut(&self, nic: usize, cut: bool) {
        if let Some(n) = self.nics.get(nic) {
            n.cut.store(cut, Ordering::SeqCst);
        }
    }

    /// Write a frame out of NIC `nic`.
    pub fn send(&self, nic: usize, f: &[u8]) -> std::io::Result<()> {
        let n = self
            .nics
            .get(nic)
            .ok_or_else(|| std::io::Error::other("no such NIC"))?;
        n.sock.send_to(f, n.qemu).map(|_| ())
    }

    /// Ping `to` on `vlan` (0 untagged) out of NIC `nic`: `count` echo
    /// requests of `size` bytes each, as fast as they go, then wait
    /// `wait` for the replies. How many came back, and how long the
    /// requests and replies took.
    pub fn ping(
        &self,
        nic: usize,
        vlan: u16,
        to: Ipv4Addr,
        count: u16,
        size: usize,
        wait: Duration,
    ) -> std::io::Result<(usize, Duration)> {
        let dst = self.resolve(nic, vlan, to)?;
        let id = 0x5153;
        {
            self.lock().replies.retain(|(i, ..)| *i != id);
        }
        let from = station_ip(vlan);
        let started = Instant::now();
        for seq in 0..count {
            let mut icmp = vec![8, 0, 0, 0];
            put_u16(&mut icmp, id);
            put_u16(&mut icmp, seq);
            icmp.resize(8 + size, 0xA5);
            let sum = checksum(&icmp);
            icmp[2..4].copy_from_slice(&sum.to_be_bytes());
            let tag = (vlan != 0).then_some(vlan);
            self.send(
                nic,
                &frame(dst, MAC, tag, 0x0800, &ipv4(from, to, 1, &icmp)),
            )?;
        }
        let deadline = Instant::now() + wait;
        loop {
            let got = self
                .lock()
                .replies
                .iter()
                .filter(|(i, ..)| *i == id)
                .count();
            if got >= usize::from(count) || Instant::now() >= deadline {
                let last = self
                    .lock()
                    .replies
                    .iter()
                    .filter(|(i, ..)| *i == id)
                    .map(|(.., at)| *at)
                    .max()
                    .unwrap_or_default();
                let took = (self.started + last).saturating_duration_since(started);
                return Ok((got, took));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// The MAC address of `ip` on `vlan`, asked by ARP when not known.
    pub fn resolve(&self, nic: usize, vlan: u16, ip: Ipv4Addr) -> std::io::Result<[u8; 6]> {
        for _ in 0..30 {
            if let Some(mac) = self.lock().neighbours.get(&(vlan, ip)) {
                return Ok(*mac);
            }
            let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 1];
            arp.extend_from_slice(&MAC);
            arp.extend_from_slice(&station_ip(vlan).octets());
            arp.extend_from_slice(&[0; 6]);
            arp.extend_from_slice(&ip.octets());
            let tag = (vlan != 0).then_some(vlan);
            self.send(nic, &frame([0xFF; 6], MAC, tag, 0x0806, &arp))?;
            thread::sleep(Duration::from_millis(200));
        }
        Err(std::io::Error::other(format!(
            "{ip} on VLAN {vlan} does not answer ARP"
        )))
    }
}

impl Drop for Station {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The station's thread on one NIC.
fn serve(i: usize, nic: &Nic, state: &Mutex<State>, started: Instant, stop: &AtomicBool) {
    let mut buf = vec![0u8; 65_536];
    while !stop.load(Ordering::SeqCst) {
        let Ok((n, _)) = nic.sock.recv_from(&mut buf) else {
            continue;
        };
        if nic.cut.load(Ordering::SeqCst) {
            continue;
        }
        let f = &buf[..n];
        let Some((vlan, ethertype, payload)) = untag(f) else {
            continue;
        };
        let (Ok(dst), Ok(src)) = (
            <[u8; 6]>::try_from(&f[0..6]),
            <[u8; 6]>::try_from(&f[6..12]),
        ) else {
            continue;
        };
        let answers = {
            let mut s = state.lock().unwrap_or_else(PoisonError::into_inner);
            if s.seen.len() < KEPT {
                s.seen.push(Seen {
                    nic: i,
                    vlan,
                    dst,
                    src,
                    ethertype,
                    len: n,
                    at: started.elapsed(),
                });
            }
            respond(&mut s, vlan, src, ethertype, payload, started)
        };
        for a in answers {
            let _ = nic.sock.send_to(&a, nic.qemu);
        }
    }
}

/// What the station says back to a frame.
fn respond(
    s: &mut State,
    vlan: Option<u16>,
    src: [u8; 6],
    ethertype: u16,
    p: &[u8],
    started: Instant,
) -> Vec<Vec<u8>> {
    let v = vlan.unwrap_or(0);
    let me = station_ip(v);
    match ethertype {
        0x0806 if p.len() >= 28 => {
            let sender = Ipv4Addr::new(p[14], p[15], p[16], p[17]);
            let target = Ipv4Addr::new(p[24], p[25], p[26], p[27]);
            if sender != Ipv4Addr::UNSPECIFIED {
                s.neighbours.insert((v, sender), src);
            }
            if p[7] == 1 && target == me {
                let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 2];
                arp.extend_from_slice(&MAC);
                arp.extend_from_slice(&me.octets());
                arp.extend_from_slice(&src);
                arp.extend_from_slice(&sender.octets());
                return vec![frame(src, MAC, vlan, 0x0806, &arp)];
            }
            Vec::new()
        }
        0x0800 if p.len() >= 20 => {
            let ihl = usize::from(p[0] & 0x0F) * 4;
            let total = usize::from(u16::from_be_bytes([p[2], p[3]])).min(p.len());
            let proto = p[9];
            let from = Ipv4Addr::new(p[12], p[13], p[14], p[15]);
            let to = Ipv4Addr::new(p[16], p[17], p[18], p[19]);
            let Some(body) = p.get(ihl..total) else {
                return Vec::new();
            };
            if from != Ipv4Addr::UNSPECIFIED {
                s.neighbours.insert((v, from), src);
            }
            match proto {
                1 if body.len() >= 8 && to == me => match body[0] {
                    8 => {
                        let mut icmp = body.to_vec();
                        icmp[0] = 0;
                        icmp[2] = 0;
                        icmp[3] = 0;
                        let sum = checksum(&icmp);
                        icmp[2..4].copy_from_slice(&sum.to_be_bytes());
                        vec![frame(src, MAC, vlan, 0x0800, &ipv4(me, from, 1, &icmp))]
                    }
                    0 => {
                        let id = u16::from_be_bytes([body[4], body[5]]);
                        let seq = u16::from_be_bytes([body[6], body[7]]);
                        s.replies.push((id, seq, started.elapsed()));
                        Vec::new()
                    }
                    _ => Vec::new(),
                },
                17 if body.len() >= 8 && s.dhcp => {
                    let dport = u16::from_be_bytes([body[2], body[3]]);
                    if dport == 67 {
                        dhcp(s, vlan, src, &body[8..])
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// A DHCP server's answer to `msg` from `client` on `vlan`.
#[allow(clippy::many_single_char_names)] // the protocol's own fields
fn dhcp(s: &mut State, vlan: Option<u16>, client: [u8; 6], msg: &[u8]) -> Vec<Vec<u8>> {
    if msg.len() < 240 || msg[0] != 1 || msg[236..240] != [99, 130, 83, 99] {
        return Vec::new();
    }
    let mut kind = 0;
    let mut at = 240;
    while at + 1 < msg.len() && msg[at] != 255 {
        if msg[at] == 0 {
            at += 1;
            continue;
        }
        let len = usize::from(msg[at + 1]);
        if msg[at] == 53 && len >= 1 {
            kind = *msg.get(at + 2).unwrap_or(&0);
        }
        at += 2 + len;
    }
    let reply = match kind {
        1 => 2,
        3 => 5,
        _ => return Vec::new(),
    };
    let v = vlan.unwrap_or(0);
    let given = s.leases.len();
    let ip = *s.leases.entry((v, client)).or_insert_with(|| {
        let [a, b, c] = subnet(v);
        Ipv4Addr::new(a, b, c, 100 + u8::try_from(given % 100).unwrap_or(0))
    });
    s.neighbours.insert((v, ip), client);
    let me = station_ip(v);
    let mut out = vec![2, 1, 6, 0];
    out.extend_from_slice(&msg[4..8]);
    out.extend_from_slice(&[0, 0, 0x80, 0]);
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&ip.octets());
    out.extend_from_slice(&me.octets());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&msg[28..44]);
    out.resize(236, 0);
    out.extend_from_slice(&[99, 130, 83, 99]);
    out.extend_from_slice(&[53, 1, reply]);
    out.extend_from_slice(&[54, 4]);
    out.extend_from_slice(&me.octets());
    out.extend_from_slice(&[51, 4, 0, 0, 0x0E, 0x10]);
    out.extend_from_slice(&[1, 4, 255, 255, 255, 0]);
    out.extend_from_slice(&[3, 4]);
    out.extend_from_slice(&me.octets());
    out.push(255);
    let mut udp = Vec::new();
    put_u16(&mut udp, 67);
    put_u16(&mut udp, 68);
    put_u16(&mut udp, u16::try_from(8 + out.len()).unwrap_or(u16::MAX));
    put_u16(&mut udp, 0);
    udp.extend_from_slice(&out);
    let packet = ipv4(me, Ipv4Addr::BROADCAST, 17, &udp);
    vec![frame([0xFF; 6], MAC, vlan, 0x0800, &packet)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discover(client: [u8; 6]) -> Vec<u8> {
        let mut m = vec![1, 1, 6, 0, 1, 2, 3, 4];
        m.resize(28, 0);
        m.extend_from_slice(&client);
        m.resize(236, 0);
        m.extend_from_slice(&[99, 130, 83, 99, 53, 1, 1, 255]);
        m
    }

    #[test]
    fn a_client_is_offered_an_address_of_its_vlan_and_keeps_it() {
        let mut s = State {
            dhcp: true,
            ..State::default()
        };
        let c = [2, 0, 0, 0, 0, 9];
        let a = dhcp(&mut s, Some(20), c, &discover(c));
        assert_eq!(a.len(), 1);
        let (vlan, ethertype, p) = untag(&a[0]).unwrap();
        assert_eq!((vlan, ethertype), (Some(20), 0x0800));
        assert_eq!(&p[16..20], &[255, 255, 255, 255]);
        assert_eq!(s.leases[&(20, c)], Ipv4Addr::new(10, 0, 20, 100));
        dhcp(&mut s, Some(20), c, &discover(c));
        assert_eq!(s.leases.len(), 1);
        assert!(dhcp(&mut s, None, c, &[0; 10]).is_empty());
        assert_eq!(station_ip(0), Ipv4Addr::new(10, 255, 0, 1));
    }

    #[test]
    fn arp_for_the_station_and_its_echo_are_answered() {
        let mut s = State::default();
        let c = [2, 0, 0, 0, 0, 7];
        let mut arp = vec![0, 1, 8, 0, 6, 4, 0, 1];
        arp.extend_from_slice(&c);
        arp.extend_from_slice(&[10, 0, 10, 5]);
        arp.extend_from_slice(&[0; 6]);
        arp.extend_from_slice(&station_ip(10).octets());
        let out = respond(&mut s, Some(10), c, 0x0806, &arp, Instant::now());
        assert_eq!(out.len(), 1);
        assert_eq!(&out[0][0..6], &c);
        assert_eq!(s.neighbours[&(10, Ipv4Addr::new(10, 0, 10, 5))], c);
        // An echo to the station comes back with a good checksum.
        let mut icmp = vec![8, 0, 0, 0, 0, 1, 0, 1, 9, 9];
        let sum = checksum(&icmp);
        icmp[2..4].copy_from_slice(&sum.to_be_bytes());
        let ip = ipv4(Ipv4Addr::new(10, 0, 10, 5), station_ip(10), 1, &icmp);
        let out = respond(&mut s, Some(10), c, 0x0800, &ip, Instant::now());
        let (_, _, p) = untag(&out[0]).unwrap();
        assert_eq!(p[20], 0);
        assert_eq!(checksum(&p[20..30]), 0);
        assert_eq!(checksum(&p[..20]), 0);
    }
}
