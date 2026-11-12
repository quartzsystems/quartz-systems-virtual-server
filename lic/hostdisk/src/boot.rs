// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The firmware side of the slot: a `Boot####` entry for each slot's
//! UKI on each load-source disk, and `BootOrder` with the default slot
//! first, on every disk, then the other. The firmware so tries the same
//! slot on the second disk before it falls to the other slot. Through
//! efivarfs; the variables' formats are QS/400's `diskimg::efi`.

use std::fs;
use std::path::Path;

use abrec::Slot;
use diskimg::efi::{self, EspLocation};

use crate::layout::{SLOT_A_PATH, SLOT_B_PATH};
use crate::record::{letter, other};

/// Where efivarfs is mounted.
pub const EFIVARS: &str = "/sys/firmware/efi/efivars";

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

/// One boot entry: its number, the file it boots if it names one, and
/// its load option.
struct Found {
    number: u16,
    path: Option<String>,
    option: Vec<u8>,
}

fn entries(vars: &Path) -> Vec<Found> {
    let mut out: Vec<Found> = fs::read_dir(vars)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let number = boot_number(&e.file_name().to_string_lossy())?;
            // The attributes come before the load option.
            let option = fs::read(e.path()).ok()?.get(4..)?.to_vec();
            Some(Found {
                number,
                path: efi::load_option_path(&option),
                option,
            })
        })
        .collect();
    out.sort_by_key(|f| f.number);
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

/// Whether a load option is on the partition `esp`: its device path
/// carries the partition's GUID.
fn on(option: &[u8], esp: &EspLocation) -> bool {
    let guid = esp.guid.to_bytes_le();
    option.windows(guid.len()).any(|w| w == guid)
}

/// The boot entry of `slot` on disk `disk` (from 0): the one there, or
/// a new one.
fn entry(
    vars: &Path,
    name: &str,
    disk: usize,
    esp: &EspLocation,
    slot: Slot,
) -> Result<u16, String> {
    let all = entries(vars);
    let wanted = option_path(slot);
    let found = all.iter().find(|f| {
        on(&f.option, esp)
            && f.path
                .as_ref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&wanted))
    });
    if let Some(found) = found {
        return Ok(found.number);
    }
    let number = (0..=u16::MAX)
        .find(|n| all.iter().all(|f| f.number != *n))
        .ok_or("no boot entry number is free")?;
    let description = if disk == 0 {
        format!("{name} slot {}", letter(slot))
    } else {
        format!("{name} slot {}, disk {}", letter(slot), disk + 1)
    };
    let option = efi::load_option(&description, esp, uki(slot));
    write(vars, &efi::boot_name(number), &option)?;
    Ok(number)
}

/// Make `slot` the one the firmware boots: each slot of each disk of
/// `esps` has an entry named after `name`, and `BootOrder` starts with
/// `slot`'s entries, then the other slot's. The new order.
pub fn make_default(
    vars: &Path,
    name: &str,
    esps: &[EspLocation],
    slot: Slot,
) -> Result<Vec<u16>, String> {
    if !vars.is_dir() {
        return Err(format!("{} is not there", vars.display()));
    }
    if esps.is_empty() {
        return Err("no load-source disk has an ESP".into());
    }
    let mut front = Vec::new();
    for slot in [slot, other(slot)] {
        for (disk, esp) in esps.iter().enumerate() {
            front.push(entry(vars, name, disk, esp, slot)?);
        }
    }
    let current = fs::read(vars.join(efi::var_name("BootOrder")))
        .map(|file| efi::parse_numbers(&file))
        .unwrap_or_default();
    let order = efi::order_with_front(&current, &front);
    if order != current {
        write(vars, "BootOrder", &efi::numbers(&order))?;
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const NAME: &str = "QS/VS";

    fn esp() -> EspLocation {
        EspLocation {
            index: 1,
            first_lba: 2048,
            sectors: 524_288,
            guid: crate::layout::fixed_guids().parts[0],
        }
    }

    fn second() -> EspLocation {
        EspLocation {
            guid: Uuid::from_u128(0x5153_5653_0000_4000_8000_0000_0000_00aa),
            ..esp()
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("hostdisk-efi-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn order(vars: &Path) -> Vec<u16> {
        efi::parse_numbers(&fs::read(vars.join(efi::var_name("BootOrder"))).unwrap())
    }

    fn paths(vars: &Path) -> Vec<(u16, Option<String>)> {
        entries(vars)
            .into_iter()
            .map(|f| (f.number, f.path))
            .collect()
    }

    #[test]
    fn a_disk_with_no_entries_gets_both_and_the_order() {
        let vars = scratch("new");
        assert_eq!(make_default(&vars, NAME, &[esp()], Slot::B), Ok(vec![0, 1]));
        assert_eq!(order(&vars), [0, 1]);
        let found = paths(&vars);
        assert_eq!(found[0], (0, Some("\\EFI\\QSVS\\B.EFI".into())));
        assert_eq!(found[1], (1, Some("\\EFI\\BOOT\\BOOTX64.EFI".into())));
        // Confirming the other slot later only turns the order round.
        assert_eq!(make_default(&vars, NAME, &[esp()], Slot::A), Ok(vec![1, 0]));
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
        assert_eq!(
            make_default(&vars, NAME, &[esp()], Slot::B),
            Ok(vec![7, 3, 0])
        );
        assert_eq!(order(&vars), [7, 3, 0]);
        assert_eq!(entries(&vars).len(), 3);
        fs::remove_dir_all(&vars).unwrap();
    }

    #[test]
    fn two_disks_have_four_entries_the_default_slot_of_each_first() {
        let vars = scratch("two");
        let both = [esp(), second()];
        assert_eq!(
            make_default(&vars, NAME, &both, Slot::A),
            Ok(vec![0, 1, 2, 3])
        );
        let found = entries(&vars);
        assert_eq!(found.len(), 4);
        // Slot A on the first disk, slot A on the second, then slot B.
        assert!(on(&found[0].option, &esp()) && on(&found[1].option, &second()));
        assert_eq!(found[1].path.as_deref(), Some("\\EFI\\BOOT\\BOOTX64.EFI"));
        assert_eq!(found[3].path.as_deref(), Some("\\EFI\\QSVS\\B.EFI"));
        // Slot B confirmed: its two entries lead, nothing is added.
        assert_eq!(
            make_default(&vars, NAME, &both, Slot::B),
            Ok(vec![2, 3, 0, 1])
        );
        assert_eq!(entries(&vars).len(), 4);
        // A disk replaced: its two entries are new, the old ones stay behind.
        let third = EspLocation {
            guid: Uuid::from_u128(0xbb),
            ..esp()
        };
        assert_eq!(
            make_default(&vars, NAME, &[esp(), third], Slot::B),
            Ok(vec![2, 4, 0, 5, 3, 1])
        );
        fs::remove_dir_all(&vars).unwrap();
    }

    #[test]
    fn no_efivarfs_is_an_error_with_its_reason() {
        let e =
            make_default(Path::new("/nonexistent/efivars"), NAME, &[esp()], Slot::A).unwrap_err();
        assert!(e.contains("not there"), "{e}");
        assert_eq!(boot_number(&efi::var_name("Boot001F")), Some(0x1f));
        assert_eq!(boot_number(&efi::var_name("BootOrder")), None);
        assert_eq!(boot_number("Boot0001-other"), None);
    }
}
