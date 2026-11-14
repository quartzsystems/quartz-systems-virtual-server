// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A logical unit of the target, of whatever kind: a disk
//! ([`super::lun`]), an optical drive ([`super::optical`]), a tape
//! drive ([`super::tape`]). Each decodes its own commands: what a
//! command means depends on the kind of device.

use std::io::{Read, Write};

use super::backing::Bounce;
use super::lun::{Counts, Failure};

/// A logical unit.
pub trait Unit: Send + Sync {
    /// Carry out the command in `cdb`. `luns` is how many LUNs the
    /// target has, for REPORT LUNS. The bytes written to `data_in`.
    fn run(
        &self,
        cdb: &[u8],
        luns: usize,
        data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        bounce: &mut Bounce,
    ) -> Result<usize, Failure>;

    /// What it was asked for so far.
    fn counts(&self) -> Counts;

    /// Blocks of its medium; 0 with none.
    fn blocks(&self) -> u64;
}

/// Write as much of `bytes` as the initiator made room for.
pub fn give(data_in: &mut dyn Write, bytes: &[u8], room: usize) -> usize {
    let mut given = 0;
    let wanted = &bytes[..bytes.len().min(room)];
    while given < wanted.len() {
        match data_in.write(&wanted[given..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => given += n,
        }
    }
    given
}

/// The REPORT LUNS list of a target of `luns` units.
pub fn report_luns(luns: usize) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + luns * 8);
    #[allow(clippy::cast_possible_truncation)] // 256 LUNs at most
    b.extend_from_slice(&(luns as u32 * 8).to_be_bytes());
    b.extend_from_slice(&[0; 4]);
    for lun in 0..luns {
        #[allow(clippy::cast_possible_truncation)]
        b.extend_from_slice(&[(lun >> 8) as u8 & 0x3F, lun as u8, 0, 0, 0, 0, 0, 0]);
    }
    b
}

/// The standard INQUIRY data: the peripheral device type, whether the
/// medium is removable, the product and the revision.
pub fn inquiry(device_type: u8, removable: bool, product: &[u8; 16]) -> Vec<u8> {
    let mut b = vec![0u8; 36];
    b[0] = device_type;
    if removable {
        b[1] = 0x80;
    }
    b[2] = 6; // SPC-4
    b[3] = 2; // response data format
    b[4] = 31; // additional length
    b[7] = 0x02; // command queuing
    b[8..16].copy_from_slice(b"QUARTZ  ");
    b[16..32].copy_from_slice(product);
    b[32..36].copy_from_slice(b"0100");
    b
}

/// The vital product data pages every kind has: the supported pages,
/// the serial number and the device identification. `None` for
/// another page.
pub fn identity_page(page: u8, serial: &str) -> Option<Vec<u8>> {
    let body: Vec<u8> = match page {
        0x00 => vec![0x00, 0x80, 0x83],
        0x80 => serial.as_bytes().to_vec(),
        0x83 => {
            let mut id = b"QUARTZ  ".to_vec();
            id.extend_from_slice(serial.as_bytes());
            #[allow(clippy::cast_possible_truncation)] // 28 at most
            let mut d = vec![0x02, 0x01, 0x00, id.len() as u8];
            d.extend_from_slice(&id);
            d
        }
        _ => return None,
    };
    #[allow(clippy::cast_possible_truncation)] // 32 at most
    let mut out = vec![0, page, 0, body.len() as u8];
    out.extend_from_slice(&body);
    Some(out)
}

/// Whether `serial` is a unit's serial number: 1 to 20 letters, digits,
/// `-` and `_`.
pub fn valid_serial(serial: &str) -> bool {
    (1..=super::lun::SERIAL_MAX).contains(&serial.len())
        && serial
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
