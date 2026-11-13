// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Making a disk a load source: the table, the ESP with both slots,
//! and the data file system. The installer writes one or two; the
//! storage service writes one when a member is added or replaced.

use std::io::{Read, Seek, SeekFrom, Write};

use appliance::LoadSourceNames;
use diskimg::fat;
use diskimg::layout::{Layout, LayoutGuids};
use diskimg::xfsfmt;
use uuid::Uuid;

use crate::layout::{ESP_BYTES, SLOT_A_PATH, install_parts};
use crate::set::BLOCK;

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A disk GUID and one for each partition, from `random` (sixteen
/// bytes a GUID, the disk's first).
pub fn guids_from(random: &[[u8; 16]; 4]) -> LayoutGuids {
    let make = |bytes: [u8; 16]| uuid::Builder::from_random_bytes(bytes).into_uuid();
    LayoutGuids {
        disk: make(random[0]),
        parts: random[1..].iter().map(|b| make(*b)).collect(),
    }
}

/// The ESP volume holding the two slots' UKIs.
pub fn esp_image(names: &LoadSourceNames, slot_a: &[u8], slot_b: &[u8]) -> Result<Vec<u8>, String> {
    let spec = fat::Spec {
        label: fat::label(names.esp_label).map_err(text)?,
        volume_id: names.esp_volume_id,
        ..fat::esp_spec(ESP_BYTES)
    };
    fat::build(&spec, &[(SLOT_A_PATH, slot_a), (names.slot_b_path, slot_b)]).map_err(text)
}

/// The layout of a load-source disk of `bytes`.
pub fn layout(names: &LoadSourceNames, bytes: u64) -> Result<Layout, String> {
    Layout::for_bytes(&install_parts(names), bytes).map_err(text)
}

/// Write the table and the ESP to `dev`, a disk of `bytes`, and clear
/// the heads of the record and data partitions. `progress` is told
/// the bytes written and the bytes in all.
pub fn write_table<D>(
    names: &LoadSourceNames,
    dev: &mut D,
    bytes: u64,
    esp: &[u8],
    guids: &LayoutGuids,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Layout, String>
where
    D: Read + Write + Seek + std::fmt::Debug,
{
    let layout = layout(names, bytes)?;
    diskimg::write_layout(dev, &layout, guids, &[(1, esp)], progress).map_err(text)?;
    Ok(layout)
}

/// Make the data file system on `dev`, which is `bytes` long: the
/// partition, or what is to be the first member of a mirrored set.
/// Its sectors are 4096 bytes, so the same file system mounts from the
/// partition and from the set's device.
pub fn format_data<D: Write + Seek>(
    names: &LoadSourceNames,
    dev: &mut D,
    bytes: u64,
    uuid: Uuid,
    now_secs: i64,
) -> Result<(), String> {
    let mut options = xfsfmt::Options::new(
        uuid,
        xfsfmt::Timestamp {
            secs: now_secs,
            nsecs: 0,
        },
    );
    options.label = names.data_name.into();
    options.sector_size = u32::try_from(BLOCK).unwrap_or(4096);
    xfsfmt::format(dev, bytes / BLOCK * BLOCK, &options).map_err(text)?;
    Ok(())
}

/// The sector size of the xfs on `dev`; `None` when there is none.
pub fn xfs_sector_size<D: Read + Seek>(dev: &mut D) -> Option<u32> {
    let mut head = [0u8; 104];
    dev.seek(SeekFrom::Start(0)).ok()?;
    dev.read_exact(&mut head).ok()?;
    (&head[..4] == b"XFSB").then(|| u32::from(u16::from_be_bytes([head[102], head[103]])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpt;
    use std::io::Cursor;

    #[test]
    fn a_prepared_disk_has_the_table_the_two_ukis_and_a_4k_xfs() {
        let names = &LoadSourceNames::DEMO;
        let bytes = crate::layout::min_disk_bytes(names) + (64 << 20);
        let guids = guids_from(&[[1; 16], [2; 16], [3; 16], [4; 16]]);
        assert_eq!(guids.parts.len(), 3);
        assert_eq!(guids.disk.get_version_num(), 4);
        let esp = esp_image(names, b"slot a", b"slot b").unwrap();
        let mut disk = Cursor::new(vec![0u8; usize::try_from(bytes).unwrap()]);
        let layout = write_table(names, &mut disk, bytes, &esp, &guids, &mut |_, _| {}).unwrap();
        let table = gpt::entries(&mut disk).unwrap();
        assert_eq!(table.len(), 3);
        assert_eq!(table[0].guid, guids.parts[0]);
        assert_eq!(
            fat::read_file(&esp, names.slot_b_path).unwrap(),
            b"slot b".to_vec()
        );

        let data = &layout.parts[2].part;
        let mut window = diskimg::Window::new(&mut disk, data.offset(), data.bytes());
        assert_eq!(xfs_sector_size(&mut window), None);
        format_data(names, &mut window, data.bytes(), Uuid::from_u128(9), 0).unwrap();
        assert_eq!(xfs_sector_size(&mut window), Some(4096));
    }
}
