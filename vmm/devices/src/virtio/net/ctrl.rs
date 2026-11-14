// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The control queue: what the guest asks of its adapter that is not a
//! frame. Every byte of a request is the guest's: a request is read to
//! [`MAX_REQUEST`] at most, a table of addresses to [`MAX_TABLE`]
//! entries, and anything that does not hold is answered with an error.
//!
//! - Receive mode (`VIRTIO_NET_CTRL_RX`): promiscuous, all multicast and
//!   the rest are recorded and answered. The switch's port decides what
//!   reaches the adapter, so nothing here changes it.
//! - Addresses (`VIRTIO_NET_CTRL_MAC`): the filter tables are recorded;
//!   an address the guest sets for itself is recorded and answered, and
//!   is not passed on: the port still passes only the profile's address.

use std::io::{Read, Write};

use virtio_queue::{Queue, QueueT};

use crate::Memory;

/// The most bytes of a request that are read.
pub const MAX_REQUEST: usize = 4096;
/// The most entries of an address table.
pub const MAX_TABLE: usize = 64;

const CLASS_RX: u8 = 0;
const CLASS_MAC: u8 = 1;
const MAC_TABLE_SET: u8 = 0;
const MAC_ADDR_SET: u8 = 1;

/// `VIRTIO_NET_OK`.
pub const OK: u8 = 0;
/// `VIRTIO_NET_ERR`.
pub const ERR: u8 = 1;

/// What the guest has asked for, recorded.
// Each flag is a receive mode of its own.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// Promiscuous mode.
    pub promiscuous: bool,
    /// Every multicast frame.
    pub all_multicast: bool,
    /// Every unicast frame.
    pub all_unicast: bool,
    /// No multicast frame.
    pub no_multicast: bool,
    /// No unicast frame.
    pub no_unicast: bool,
    /// No broadcast frame.
    pub no_broadcast: bool,
    /// The address the guest gave itself, when it did.
    pub mac: Option<[u8; 6]>,
    /// The unicast filter table.
    pub unicast: Vec<[u8; 6]>,
    /// The multicast filter table.
    pub multicast: Vec<[u8; 6]>,
}

/// A table of addresses at the front of `data`: what it holds and the
/// rest of `data`.
fn table(data: &[u8]) -> Option<(Vec<[u8; 6]>, &[u8])> {
    let count = u32::from_le_bytes(data.get(..4)?.try_into().ok()?);
    let count = usize::try_from(count).ok().filter(|c| *c <= MAX_TABLE)?;
    let bytes = count.checked_mul(6)?;
    let entries = data.get(4..4 + bytes)?;
    let rest = data.get(4 + bytes..)?;
    let (macs, _) = entries.as_chunks::<6>();
    let macs = macs.to_vec();
    Some((macs, rest))
}

/// Carry out one request, its class, command and data: the answer.
pub fn command(asked: &mut Asked, request: &[u8]) -> u8 {
    let (Some(&class), Some(&cmd)) = (request.first(), request.get(1)) else {
        return ERR;
    };
    let data = request.get(2..).unwrap_or(&[]);
    match (class, cmd) {
        (CLASS_RX, 0..=5) => {
            let Some(&on) = data.first() else { return ERR };
            let on = on != 0;
            match cmd {
                0 => asked.promiscuous = on,
                1 => asked.all_multicast = on,
                2 => asked.all_unicast = on,
                3 => asked.no_multicast = on,
                4 => asked.no_unicast = on,
                _ => asked.no_broadcast = on,
            }
            OK
        }
        (CLASS_MAC, MAC_TABLE_SET) => {
            let Some((unicast, rest)) = table(data) else {
                return ERR;
            };
            let Some((multicast, _)) = table(rest) else {
                return ERR;
            };
            asked.unicast = unicast;
            asked.multicast = multicast;
            OK
        }
        (CLASS_MAC, MAC_ADDR_SET) => {
            match data.get(..6).and_then(|m| <[u8; 6]>::try_from(m).ok()) {
                Some(mac) if mac[0] & 1 == 0 => {
                    asked.mac = Some(mac);
                    OK
                }
                _ => ERR,
            }
        }
        _ => ERR,
    }
}

/// Serve every request the guest has put in the control queue. Whether
/// any was; `said` is told each address the guest gave itself.
pub fn serve(
    queue: &mut Queue,
    memory: &Memory,
    asked: &mut Asked,
    said: &mut dyn FnMut([u8; 6]),
) -> bool {
    let mut any = false;
    let mut request = vec![0u8; MAX_REQUEST];
    while let Some(chain) = queue.pop_descriptor_chain(memory) {
        let head = chain.head_index();
        let mut used = 0u32;
        if let (Ok(mut reader), Ok(mut writer)) =
            (chain.clone().reader(memory), chain.writer(memory))
        {
            let want = reader.available_bytes().min(MAX_REQUEST);
            let answer = match reader.read_exact(&mut request[..want]) {
                Ok(()) => {
                    let before = asked.mac;
                    let answer = command(asked, &request[..want]);
                    if let Some(mac) = asked.mac.filter(|m| before != Some(*m)) {
                        said(mac);
                    }
                    answer
                }
                Err(_) => ERR,
            };
            if writer.available_bytes() >= 1 && writer.write_all(&[answer]).is_ok() {
                used = 1;
            }
        }
        if queue.add_used(memory, head, used).is_err() {
            break;
        }
        any = true;
    }
    any
}

/// Take every frame the guest has put in a transmit queue and send it
/// nowhere: an adapter without link. Whether any was.
pub fn drain(queue: &mut Queue, memory: &Memory) -> bool {
    let mut any = false;
    while let Some(chain) = queue.pop_descriptor_chain(memory) {
        if queue.add_used(memory, chain.head_index(), 0).is_err() {
            break;
        }
        any = true;
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::scsi::testing::{Guest, chain};
    use vm_memory::{Bytes, GuestAddress};

    #[test]
    fn receive_modes_and_tables_are_recorded_and_answered() {
        let mut asked = Asked::default();
        assert_eq!(command(&mut asked, &[0, 0, 1]), OK);
        assert!(asked.promiscuous);
        assert_eq!(command(&mut asked, &[0, 1, 1]), OK);
        assert_eq!(command(&mut asked, &[0, 0, 0]), OK);
        assert!(!asked.promiscuous && asked.all_multicast);
        assert_eq!(command(&mut asked, &[0, 6, 1]), ERR);
        assert_eq!(command(&mut asked, &[0, 0]), ERR);
        // One unicast entry, two multicast ones.
        let mut t = vec![1, 0];
        t.extend_from_slice(&1u32.to_le_bytes());
        t.extend_from_slice(&[2, 0, 0, 0, 0, 1]);
        t.extend_from_slice(&2u32.to_le_bytes());
        t.extend_from_slice(&[1, 0, 0x5E, 0, 0, 1, 1, 0, 0x5E, 0, 0, 2]);
        assert_eq!(command(&mut asked, &t), OK);
        assert_eq!(asked.unicast, [[2, 0, 0, 0, 0, 1]]);
        assert_eq!(asked.multicast.len(), 2);
        // A count beyond the table, or beyond the bytes there are.
        let mut big = vec![1, 0];
        big.extend_from_slice(&65u32.to_le_bytes());
        assert_eq!(command(&mut asked, &big), ERR);
        let mut short = vec![1, 0];
        short.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(command(&mut asked, &short), ERR);
        assert_eq!(command(&mut asked, &t[..t.len() - 1]), ERR);
        assert_eq!(command(&mut asked, &[]), ERR);
        assert_eq!(command(&mut asked, &[9, 9]), ERR);
    }

    #[test]
    fn an_address_the_guest_gives_itself_is_recorded_not_passed_on() {
        let mut guest = Guest::new(16);
        guest
            .memory
            .write_slice(&[1, 1, 2, 0x5A, 0, 0, 0, 9], GuestAddress(0x8000))
            .unwrap();
        chain(&mut guest, &[(0x8000, 8)], &[(0x9000, 1)]);
        // A group address is refused.
        guest
            .memory
            .write_slice(&[1, 1, 3, 0, 0, 0, 0, 1], GuestAddress(0xA000))
            .unwrap();
        chain(&mut guest, &[(0xA000, 8)], &[(0xB000, 1)]);
        // No room for the answer: used with nothing written.
        chain(&mut guest, &[(0x8000, 8)], &[]);
        let mut asked = Asked::default();
        let mut told = Vec::new();
        assert!(serve(
            &mut guest.queue,
            &guest.memory,
            &mut asked,
            &mut |m| told.push(m)
        ));
        assert_eq!(asked.mac, Some([2, 0x5A, 0, 0, 0, 9]));
        assert_eq!(told, [[2, 0x5A, 0, 0, 0, 9]]);
        assert_eq!(guest.used(), [(0, 1), (2, 1), (4, 0)]);
        let mut ack = [9u8; 1];
        guest
            .memory
            .read_slice(&mut ack, GuestAddress(0x9000))
            .unwrap();
        assert_eq!(ack, [OK]);
        guest
            .memory
            .read_slice(&mut ack, GuestAddress(0xB000))
            .unwrap();
        assert_eq!(ack, [ERR]);
    }

    #[test]
    fn an_adapter_without_link_takes_its_frames_and_sends_them_nowhere() {
        let mut guest = Guest::new(16);
        chain(&mut guest, &[(0x8000, 64)], &[]);
        chain(&mut guest, &[(0x9000, 64)], &[]);
        assert!(drain(&mut guest.queue, &guest.memory));
        assert_eq!(guest.used(), [(0, 0), (1, 0)]);
        assert!(!drain(&mut guest.queue, &guest.memory));
    }
}
