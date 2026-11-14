// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A bridge's forwarding table: a static entry for a partition's
//! address on its port and VLAN, and the table as the kernel has it.

#[allow(clippy::wildcard_imports)] // the numbers module
use crate::consts::*;
use netlink::codec::{
    Attrs, Builder, NLM_F_CREATE, NLM_F_DUMP, NLM_F_REPLACE, NLM_F_REQUEST, ne_u16, ne_u32,
};
use netlink::route::consts::{NDMSG_LEN, RTM_DELNEIGH, RTM_GETNEIGH, RTM_NEWNEIGH};
use netlink::{Error, Result};

fn ndmsg(port: u32, state: u16, flags: u8) -> [u8; NDMSG_LEN] {
    let mut b = [0u8; NDMSG_LEN];
    b[0] = AF_BRIDGE;
    b[4..8].copy_from_slice(&port.to_ne_bytes());
    b[8..10].copy_from_slice(&state.to_ne_bytes());
    b[10] = flags;
    b
}

/// Add a static entry: `mac` in `vlan` is on the port `port`; or delete
/// it.
pub fn static_request(port: u32, mac: [u8; 6], vlan: u16, add: bool) -> Result<Builder> {
    if !(1..=4094).contains(&vlan) {
        return Err(Error::Invalid(format!("VLAN id {vlan}")));
    }
    let mut b = if add {
        Builder::new(RTM_NEWNEIGH, NLM_F_REQUEST | NLM_F_CREATE | NLM_F_REPLACE)
    } else {
        Builder::new(RTM_DELNEIGH, NLM_F_REQUEST)
    };
    b.bytes(&ndmsg(port, NUD_NOARP, NTF_MASTER))
        .attr(NDA_LLADDR, &mac)
        .u16(NDA_VLAN, vlan);
    Ok(b)
}

/// Dump the forwarding table of the bridge `bridge`.
pub fn dump_request(bridge: u32) -> Builder {
    let mut b = Builder::new(RTM_GETNEIGH, NLM_F_REQUEST | NLM_F_DUMP);
    b.bytes(&ndmsg(0, 0, 0)).u32(NDA_MASTER, bridge);
    b
}

/// How an entry came to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Put there: a partition's address.
    Static,
    /// The address of a link of the host itself.
    Local,
    /// Learned from traffic.
    Learned,
}

impl Origin {
    /// In a word.
    pub fn word(self) -> &'static str {
        match self {
            Origin::Static => "static",
            Origin::Local => "local",
            Origin::Learned => "learned",
        }
    }
}

/// One entry of a forwarding table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FdbEntry {
    /// The port it points to.
    pub port: u32,
    /// The bridge.
    pub master: u32,
    /// The address.
    pub mac: [u8; 6],
    /// Its VLAN; 0 for an entry of no VLAN.
    pub vlan: u16,
    /// How it came to be.
    pub origin: Origin,
}

/// Parse one reply of [`dump_request`]. `None` for an entry that is not
/// a bridge's, or not an Ethernet address.
pub fn parse(payload: &[u8]) -> Result<Option<FdbEntry>> {
    if payload.len() < NDMSG_LEN {
        return Err(Error::Malformed("ndmsg"));
    }
    if payload[0] != AF_BRIDGE {
        return Ok(None);
    }
    let port = ne_u32(payload, 4).unwrap_or(0);
    let state = ne_u16(payload, 8).unwrap_or(0);
    let flags = payload[10];
    let attrs = Attrs::new(&payload[NDMSG_LEN..]);
    let Some(mac) = attrs
        .get(NDA_LLADDR)
        .and_then(|a| <[u8; 6]>::try_from(a.data).ok())
    else {
        return Ok(None);
    };
    // An entry of the port's own device (NTF_SELF) is not the bridge's.
    if flags & NTF_SELF != 0 && flags & NTF_MASTER == 0 {
        return Ok(None);
    }
    let origin = if state & NUD_PERMANENT != 0 {
        Origin::Local
    } else if state & NUD_NOARP != 0 && flags & NTF_EXT_LEARNED == 0 {
        Origin::Static
    } else {
        Origin::Learned
    };
    Ok(Some(FdbEntry {
        port,
        master: attrs.get(NDA_MASTER).and_then(|a| a.u32()).unwrap_or(0),
        mac,
        vlan: attrs.get(NDA_VLAN).and_then(|a| a.u16()).unwrap_or(0),
        origin,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlink::codec::NLMSG_HDRLEN;

    #[test]
    fn a_static_entry_is_noarp_on_the_master_and_parses_back() {
        let mac = [0x02, 0x51, 0x56, 0, 1, 2];
        let msg = static_request(9, mac, 10, true).unwrap().finish(1);
        assert_eq!(u16::from_ne_bytes([msg[4], msg[5]]), RTM_NEWNEIGH);
        let payload = &msg[NLMSG_HDRLEN..];
        let mut reply = payload.to_vec();
        let mut b = Builder::new(0, 0);
        b.u32(NDA_MASTER, 4);
        reply.extend_from_slice(&b.finish(0)[NLMSG_HDRLEN..]);
        let entry = parse(&reply).unwrap().unwrap();
        assert_eq!(
            entry,
            FdbEntry {
                port: 9,
                master: 4,
                mac,
                vlan: 10,
                origin: Origin::Static
            }
        );
        assert!(static_request(9, mac, 0, true).is_err());
        let del = static_request(9, mac, 10, false).unwrap().finish(2);
        assert_eq!(u16::from_ne_bytes([del[4], del[5]]), RTM_DELNEIGH);
    }

    #[test]
    fn learned_and_local_entries_are_told_apart() {
        let mut learned = ndmsg(3, 0x02, NTF_MASTER).to_vec();
        let mut b = Builder::new(0, 0);
        b.attr(NDA_LLADDR, &[0x52, 0x54, 0, 0, 0, 1])
            .u16(NDA_VLAN, 20);
        let tail = b.finish(0)[NLMSG_HDRLEN..].to_vec();
        learned.extend_from_slice(&tail);
        assert_eq!(parse(&learned).unwrap().unwrap().origin, Origin::Learned);
        let mut local = ndmsg(3, NUD_PERMANENT, NTF_MASTER).to_vec();
        local.extend_from_slice(&tail);
        assert_eq!(parse(&local).unwrap().unwrap().origin, Origin::Local);
        let mut own = ndmsg(3, NUD_PERMANENT, NTF_SELF).to_vec();
        own.extend_from_slice(&tail);
        assert_eq!(parse(&own).unwrap(), None);
        assert!(parse(&[0u8; 4]).is_err());
    }
}
