// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The firmware side of the slot: a `Boot####` entry for each slot's
//! UKI and `BootOrder` with the confirmed slot first, through efivarfs.
//! The variables' formats are QS/400's `diskimg::efi`.

use std::fs;
use std::path::Path;

use abrec::Slot;
use diskimg::efi::{self, EspLocation};
use hostdisk::layout::{SLOT_A_PATH, SLOT_B_PATH};
use hostdisk::record::{letter, other};

/// A slot's UKI on the ESP.
fn uki(slot: Slot) -> &'static str {
    match slot {
        Slot::A => SLOT_A_PATH,
        Slot::B => SLOT_B_PATH,
    }
}

/// The path as a load option has it: backslashes, one leading.
fn option_path(slot: Slot) -> String {
    format!("\\{}", uki(slot).replace('/', "\\"))
}

/// The number in a `Boot####-<guid>` file name.
fn boot_number(file: &str) -> Option<u16> {
    let hex = file
        .strip_suffix(efi::GLOBAL)?
        .strip_suffix('-')?
        .strip_prefix("Boot")?;
    (hex.len() == 4 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u16::from_str_radix(hex, 16).ok())
        .flatten()
}

/// Every boot entry: its number and the file it boots, if it names one.
fn entries(vars: &Path) -> Vec<(u16, Option<String>)> {
    let mut out: Vec<(u16, Option<String>)> = fs::read_dir(vars)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let number = boot_number(&e.file_name().to_string_lossy())?;
            // The attributes come before the load option.
            let bytes = fs::read(e.path()).ok()?;
            Some((number, efi::load_option_path(bytes.get(4..)?)))
        })
        .collect();
    out.sort();
    out
}

/// Write a variable. efivarfs makes its files immutable; that is
/// lifted for the write, which must be one `write` of the whole value.
fn write(vars: &Path, name: &str, data: &[u8]) -> Result<(), String> {
    let path = vars.join(efi::var_name(name));
    #[cfg(target_os = "linux")]
    if let Ok(file) = fs::File::open(&path) {
        use rustix::fs::{IFlags, ioctl_getflags, ioctl_setflags};
        if let Ok(flags) = ioctl_getflags(&file) {
            let _ = ioctl_setflags(&file, flags.difference(IFlags::IMMUTABLE));
        }
    }
    fs::write(&path, efi::file_bytes(data)).map_err(|e| format!("{name}: {e}"))
}

/// The boot entry of `slot`: the one there, or a new one.
fn entry(vars: &Path, esp: &EspLocation, slot: Slot) -> Result<u16, String> {
    let all = entries(vars);
    let wanted = option_path(slot);
    let found = all.iter().find(|(_, path)| {
        path.as_ref()
            .is_some_and(|p| p.eq_ignore_ascii_case(&wanted))
    });
    if let Some((number, _)) = found {
        return Ok(*number);
    }
    let number = (0..=u16::MAX)
        .find(|n| all.iter().all(|(used, _)| used != n))
        .ok_or("no boot entry number is free")?;
    let description = format!("{} slot {}", release::NAME, letter(slot));
    let option = efi::load_option(&description, esp, uki(slot));
    write(vars, &efi::boot_name(number), &option)?;
    Ok(number)
}

/// Make `slot` the one the firmware boots: both slots have an entry,
/// and `BootOrder` starts with `slot`'s, then the other's. The new
/// order.
pub fn make_default(vars: &Path, esp: &EspLocation, slot: Slot) -> Result<Vec<u16>, String> {
    if !vars.is_dir() {
        return Err(format!("{} is not there", vars.display()));
    }
    let first = entry(vars, esp, slot)?;
    let second = entry(vars, esp, other(slot))?;
    let current = fs::read(vars.join(efi::var_name("BootOrder")))
        .map(|file| efi::parse_numbers(&file))
        .unwrap_or_default();
    let order = efi::order_with_front(&current, &[first, second]);
    if order != current {
        write(vars, "BootOrder", &efi::numbers(&order))?;
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esp() -> EspLocation {
        EspLocation {
            index: 1,
            first_lba: 2048,
            sectors: 524_288,
            guid: hostdisk::layout::fixed_guids().parts[0],
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qsvsinit-efi-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn order(vars: &Path) -> Vec<u16> {
        efi::parse_numbers(&fs::read(vars.join(efi::var_name("BootOrder"))).unwrap())
    }

    #[test]
    fn a_disk_with_no_entries_gets_both_and_the_order() {
        let vars = scratch("new");
        assert_eq!(make_default(&vars, &esp(), Slot::B), Ok(vec![0, 1]));
        assert_eq!(order(&vars), [0, 1]);
        let found = entries(&vars);
        assert_eq!(found[0], (0, Some("\\EFI\\QSVS\\B.EFI".into())));
        assert_eq!(found[1], (1, Some("\\EFI\\BOOT\\BOOTX64.EFI".into())));
        // Confirming the other slot later only turns the order round.
        assert_eq!(make_default(&vars, &esp(), Slot::A), Ok(vec![1, 0]));
        assert_eq!(entries(&vars).len(), 2);
        fs::remove_dir_all(&vars).unwrap();
    }

    #[test]
    fn existing_entries_are_found_and_others_kept_behind() {
        let vars = scratch("existing");
        let put = |name: &str, data: &[u8]| {
            fs::write(vars.join(efi::var_name(name)), efi::file_bytes(data)).unwrap();
        };
        // The firmware's own entry, slot A and slot B as an installer
        // or the harness wrote them, and the order with A first.
        put("Boot0000", b"\x01\x00\x00\x00\x00\x00U\x00\x00\x00");
        put(
            "Boot0003",
            &efi::load_option("QS/VS slot A", &esp(), SLOT_A_PATH),
        );
        put(
            "Boot0007",
            &efi::load_option("QS/VS slot B", &esp(), SLOT_B_PATH),
        );
        put("BootOrder", &efi::numbers(&[3, 7, 0]));
        put("BootNext", &efi::numbers(&[7]));
        assert_eq!(make_default(&vars, &esp(), Slot::B), Ok(vec![7, 3, 0]));
        assert_eq!(order(&vars), [7, 3, 0]);
        assert_eq!(entries(&vars).len(), 3);
        fs::remove_dir_all(&vars).unwrap();
    }

    #[test]
    fn no_efivarfs_is_an_error_with_its_reason() {
        let e = make_default(Path::new("/nonexistent/efivars"), &esp(), Slot::A).unwrap_err();
        assert!(e.contains("not there"), "{e}");
        assert_eq!(boot_number(&efi::var_name("Boot001F")), Some(0x1f));
        assert_eq!(boot_number(&efi::var_name("BootOrder")), None);
        assert_eq!(boot_number("Boot0001-other"), None);
    }
}
