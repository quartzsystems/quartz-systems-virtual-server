// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Writing UEFI variables into a copy of OVMF's variable store, so a
//! test boot goes by a firmware boot entry (`Boot0000` and `BootOrder`)
//! the way an installed host's does. The store is a firmware volume
//! holding an authenticated variable store (EDK II's
//! `VariableFormat.h`); variables are appended after the ones there.

use uuid::{Uuid, uuid};

/// The firmware volume header's signature, at offset 40.
const FV_SIGNATURE: &[u8; 4] = b"_FVH";
/// The authenticated variable store's GUID.
const AUTH_STORE: Uuid = uuid!("aaf32c78-947b-439a-a180-2e144ec37792");
/// The EFI global variable GUID.
const GLOBAL: Uuid = uuid!("8be4df61-93ca-11d2-aa0d-00e098032b8c");
/// Bytes of the variable store header.
const STORE_HEADER: usize = 28;
/// A formatted, healthy store.
const STORE_FORMATTED: u8 = 0x5A;
const STORE_HEALTHY: u8 = 0xFE;
/// What every variable header starts with.
const START_ID: u16 = 0x55AA;
/// Bytes of an authenticated variable header.
const VAR_HEADER: usize = 60;
/// A variable that was added and is valid.
const VAR_ADDED: u8 = 0x3F;
/// Non-volatile, boot service and runtime access.
const ATTRS: u32 = 0x7;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    let v = u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?);
    usize::try_from(v).ok()
}

fn align4(n: usize) -> usize {
    n.div_ceil(4) * 4
}

/// The variable area of `store`: where its first variable is and where
/// it ends.
fn area(store: &[u8]) -> Result<(usize, usize), String> {
    if store.get(40..44) != Some(FV_SIGNATURE) {
        return Err("not a firmware volume (no _FVH)".into());
    }
    let header = usize::from(u16_at(store, 48).ok_or("short firmware volume header")?);
    let head = store
        .get(header..header + STORE_HEADER)
        .ok_or("no variable store header")?;
    if head[..16] != AUTH_STORE.to_bytes_le() {
        return Err("not an authenticated variable store".into());
    }
    if (head[20], head[21]) != (STORE_FORMATTED, STORE_HEALTHY) {
        return Err("the variable store is not formatted and healthy".into());
    }
    let size = u32_at(head, 16).ok_or("no variable store size")?;
    let end = header + size;
    if size < STORE_HEADER || end > store.len() {
        return Err("the variable store's size is out of range".into());
    }
    Ok((header + STORE_HEADER, end))
}

/// The offset after the last variable in the area.
fn free(store: &[u8], start: usize, end: usize) -> Result<usize, String> {
    let mut at = start;
    while at + VAR_HEADER <= end && u16_at(store, at) == Some(START_ID) {
        let name = u32_at(store, at + 36).ok_or("short variable header")?;
        let data = u32_at(store, at + 40).ok_or("short variable header")?;
        at = align4(at + VAR_HEADER + name + data);
    }
    if at > end {
        return Err("a variable runs past the store".into());
    }
    Ok(at)
}

/// One variable as stored: header, UCS-2 name, data.
fn variable(name: &str, data: &[u8]) -> Vec<u8> {
    let name: Vec<u8> = name
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect();
    let len32 = |n: usize| {
        u32::try_from(n)
            .expect("a variable under 4 GiB")
            .to_le_bytes()
    };
    let mut v = Vec::with_capacity(VAR_HEADER + name.len() + data.len());
    v.extend_from_slice(&START_ID.to_le_bytes());
    v.extend_from_slice(&[VAR_ADDED, 0]);
    v.extend_from_slice(&ATTRS.to_le_bytes());
    v.extend_from_slice(&[0; 8]); // monotonic count
    v.extend_from_slice(&[0; 16]); // time stamp
    v.extend_from_slice(&[0; 4]); // public key index
    v.extend_from_slice(&len32(name.len()));
    v.extend_from_slice(&len32(data.len()));
    v.extend_from_slice(&GLOBAL.to_bytes_le());
    v.extend_from_slice(&name);
    v.extend_from_slice(data);
    v
}

/// `store` with the global variables `vars` (name, data) appended.
pub fn with_variables(store: &[u8], vars: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let (start, end) = area(store)?;
    let mut at = free(store, start, end)?;
    let mut out = store.to_vec();
    for (name, data) in vars {
        let bytes = variable(name, data);
        let next = at + bytes.len();
        if next > end {
            return Err(format!("no room for {name} in the variable store"));
        }
        if out[at..next].iter().any(|b| *b != 0xFF) {
            return Err("the variable store's free space is not erased".into());
        }
        out[at..next].copy_from_slice(&bytes);
        at = align4(next);
    }
    Ok(out)
}

/// `store` with one boot entry, `Boot0000`, first in `BootOrder`.
pub fn with_boot_entry(store: &[u8], load_option: &[u8]) -> Result<Vec<u8>, String> {
    with_variables(
        store,
        &[
            ("Boot0000", load_option),
            ("BootOrder", &0u16.to_le_bytes()),
        ],
    )
}

/// The global variables in `store`, by name. For the tests.
pub fn variables(store: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let (start, end) = area(store)?;
    let stop = free(store, start, end)?;
    let mut out = Vec::new();
    let mut at = start;
    while at < stop {
        let name_len = u32_at(store, at + 36).ok_or("short variable header")?;
        let data_len = u32_at(store, at + 40).ok_or("short variable header")?;
        let name_at = at + VAR_HEADER;
        let units: Vec<u16> = store[name_at..name_at + name_len]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|u| *u != 0)
            .collect();
        let data_at = name_at + name_len;
        out.push((
            String::from_utf16_lossy(&units),
            store[data_at..data_at + data_len].to_vec(),
        ));
        at = align4(data_at + data_len);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty store as OVMF ships it: a 0x48-byte volume header, the
    /// store header, erased flash.
    fn empty() -> Vec<u8> {
        let mut s = vec![0xFFu8; 0x2000];
        s[..0x48].fill(0);
        s[40..44].copy_from_slice(FV_SIGNATURE);
        s[48..50].copy_from_slice(&0x48u16.to_le_bytes());
        s[0x48..0x58].copy_from_slice(&AUTH_STORE.to_bytes_le());
        s[0x58..0x5C].copy_from_slice(&0x1000u32.to_le_bytes());
        s[0x5C] = STORE_FORMATTED;
        s[0x5D] = STORE_HEALTHY;
        s[0x5E..0x64].fill(0);
        s
    }

    #[test]
    fn a_boot_entry_lands_after_the_header_and_reads_back() {
        let out = with_boot_entry(&empty(), b"option").unwrap();
        assert_eq!(out.len(), 0x2000);
        // The first variable starts right after the store header.
        assert_eq!(out[0x64..0x68], [0xAA, 0x55, VAR_ADDED, 0]);
        assert_eq!(out[0x68..0x6C], ATTRS.to_le_bytes());
        let vars = variables(&out).unwrap();
        assert_eq!(
            vars,
            [
                ("Boot0000".to_string(), b"option".to_vec()),
                ("BootOrder".to_string(), vec![0, 0]),
            ]
        );
        // Headers are four-byte aligned.
        let second = align4(0x64 + VAR_HEADER + 18 + 6);
        assert_eq!(out[second..second + 2], [0xAA, 0x55]);
    }

    #[test]
    fn variables_go_after_the_ones_already_there() {
        let once = with_variables(&empty(), &[("Lang", b"eng")]).unwrap();
        let twice = with_boot_entry(&once, b"x").unwrap();
        let names: Vec<String> = variables(&twice)
            .unwrap()
            .into_iter()
            .map(|v| v.0)
            .collect();
        assert_eq!(names, ["Lang", "Boot0000", "BootOrder"]);
    }

    #[test]
    fn what_is_not_a_store_is_refused() {
        assert!(with_boot_entry(&[0u8; 4096], b"x").is_err());
        let mut plain = empty();
        plain[0x48] ^= 1;
        assert!(with_boot_entry(&plain, b"x").is_err());
        // No room: the store's size covers the header alone.
        let mut full = empty();
        full[0x58..0x5C].copy_from_slice(&64u32.to_le_bytes());
        assert!(
            with_boot_entry(&full, b"x")
                .unwrap_err()
                .contains("no room")
        );
    }
}
