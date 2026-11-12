// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The ACPI power button: an input device that reports `KEY_POWER`.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;

use rustix::fs::OFlags;

/// `EV_KEY`.
const EV_KEY: u16 = 1;
/// `KEY_POWER`.
const KEY_POWER: u16 = 116;
/// Bytes of a `struct input_event` on `x86_64`: a 16-byte time, the
/// type, the code and the value.
const EVENT: usize = 24;

/// Whether a `capabilities/key` bitmap (hex words, the highest first)
/// has `KEY_POWER`.
pub fn has_power_key(capabilities: &str) -> bool {
    let words: Vec<&str> = capabilities.split_ascii_whitespace().rev().collect();
    let bit = usize::from(KEY_POWER);
    words
        .get(bit / 64)
        .and_then(|w| u64::from_str_radix(w, 16).ok())
        .is_some_and(|w| w >> (bit % 64) & 1 == 1)
}

/// Every input device with a power key, opened for reading without
/// blocking.
pub fn open() -> Vec<File> {
    let mut names: Vec<String> = fs::read_dir("/sys/class/input")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("event"))
        .collect();
    names.sort();
    let flags = i32::try_from(OFlags::NONBLOCK.bits()).unwrap_or(0);
    names
        .iter()
        .filter(|name| {
            fs::read_to_string(format!("/sys/class/input/{name}/device/capabilities/key"))
                .is_ok_and(|c| has_power_key(&c))
        })
        .filter_map(|name| {
            OpenOptions::new()
                .read(true)
                .custom_flags(flags)
                .open(format!("/dev/input/{name}"))
                .ok()
        })
        .collect()
}

/// Read what `device` has to report. True when the power key went down.
pub fn pressed(mut device: &File) -> bool {
    let mut pressed = false;
    let mut event = [0u8; EVENT];
    while let Ok(EVENT) = device.read(&mut event) {
        let kind = u16::from_ne_bytes([event[16], event[17]]);
        let code = u16::from_ne_bytes([event[18], event[19]]);
        let value = i32::from_ne_bytes([event[20], event[21], event[22], event[23]]);
        pressed |= kind == EV_KEY && code == KEY_POWER && value == 1;
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_power_key_is_bit_116_of_the_bitmap() {
        // The ACPI power button, and a keyboard that has the key too.
        assert!(has_power_key("10000000000000 0\n"));
        assert!(has_power_key(
            "402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe"
        ));
        // A lid switch; a mouse.
        assert!(!has_power_key("0\n"));
        assert!(!has_power_key("70000 0 0 0 0"));
        assert!(!has_power_key(""));
    }
}
