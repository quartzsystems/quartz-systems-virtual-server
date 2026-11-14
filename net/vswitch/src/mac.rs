// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition adapter's MAC address. One a profile does not give is
//! made when the adapter is first saved, and kept in the profile:
//! locally administered and unicast, three bytes of a prefix the host
//! derives once from its serial number, then the partition's ID in two
//! bytes and the slot in one. It never changes afterwards.

/// A MAC address.
pub type Mac = [u8; 6];

/// `xx:xx:xx:xx:xx:xx`, lower case.
pub fn text(mac: &Mac) -> String {
    mac.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// From six pairs of hexadecimal digits with colons.
pub fn parse(text: &str) -> Option<Mac> {
    let mut mac = [0u8; 6];
    let mut parts = text.split(':');
    for byte in &mut mac {
        let part = parts.next()?;
        if part.len() != 2 || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        *byte = u8::from_str_radix(part, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

/// Whether it is a group (multicast or broadcast) address.
pub fn is_group(mac: &Mac) -> bool {
    mac[0] & 1 != 0
}

/// Whether it is all zeros.
pub fn is_zero(mac: &Mac) -> bool {
    mac.iter().all(|b| *b == 0)
}

/// The host's prefix: three bytes of a hash of its serial number, the
/// first locally administered and unicast. The same serial number gives
/// the same prefix; it is kept in state once made, so a serial number
/// that reads otherwise later changes nothing.
pub fn prefix_of(serial: &str) -> [u8; 3] {
    // FNV-1a, 32 bits: fixed, and enough to spread hosts apart.
    let mut h: u32 = 0x811C_9DC5;
    for b in serial.trim().bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let [a, b, c, _] = h.to_be_bytes();
    [(a & 0xFC) | 0x02, b, c]
}

/// The address made for slot `slot` of partition `id` under `prefix`.
pub fn made(prefix: [u8; 3], id: u32, slot: u32) -> Mac {
    let [_, _, hi, lo] = id.to_be_bytes();
    let slot = u8::try_from(slot).unwrap_or(u8::MAX);
    [prefix[0], prefix[1], prefix[2], hi, lo, slot]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_made_address_is_local_unicast_and_names_the_partition_and_slot() {
        let p = prefix_of("QSVS0001");
        assert_eq!(p, prefix_of(" QSVS0001\n"));
        assert_ne!(p, prefix_of("QSVS0002"));
        let mac = made(p, 999, 2);
        assert_eq!(mac[0] & 0x03, 0x02, "locally administered and unicast");
        assert_eq!(&mac[3..], &[0x03, 0xE7, 0x02]);
        assert!(!is_group(&mac));
        assert_ne!(made(p, 1, 2), made(p, 1, 3));
        assert_ne!(made(p, 1, 2), made(p, 2, 2));
    }

    #[test]
    fn an_address_reads_as_six_pairs() {
        let mac = parse("02:AB:cd:00:01:ff").unwrap();
        assert_eq!(text(&mac), "02:ab:cd:00:01:ff");
        assert!(is_group(&parse("01:00:5e:00:00:01").unwrap()));
        assert!(is_group(&[0xFF; 6]));
        assert!(is_zero(&[0; 6]));
        for bad in [
            "",
            "02:ab:cd:00:01",
            "02:ab:cd:00:01:ff:00",
            "2:ab:cd:00:01:ff",
            "+2:ab:cd:00:01:ff",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
