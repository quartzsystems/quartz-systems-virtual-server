// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partitions, their GPT types and the fixed GUIDs of a built image.

use diskimg::layout::{Content, ESP_TYPE_GUID, LayoutGuids, PartDesc, Size};
use uuid::{Uuid, uuid};

const MIB: u64 = 1024 * 1024;

/// GPT type of the A/B record partition. QS/VS's own.
pub const RECORD_TYPE_GUID: Uuid = uuid!("1f072256-9b60-4f79-a93e-f2af75b72775");
/// GPT type of the `QSVSDATA` partition. QS/VS's own.
pub const DATA_TYPE_GUID: Uuid = uuid!("5a62035d-0a21-4e5c-8bd1-e3908ef0dfb9");
/// The xfs UUID of a built image's `QSVSDATA`.
pub const DATA_FS_UUID: Uuid = uuid!("fe6b2094-8863-4006-8bcd-fadcdf206e94");

/// GPT name of the ESP.
pub const ESP_NAME: &str = "QSVS ESP";
/// GPT name of the A/B record partition.
pub const RECORD_NAME: &str = "QSVS-AB";
/// GPT name and xfs label of the data partition.
pub const DATA_NAME: &str = "QSVSDATA";

/// The ESP vendor directory.
pub const VENDOR: &str = "QSVS";
/// Slot A's UKI on the ESP: the removable-media default boot path.
pub const SLOT_A_PATH: &str = "EFI/BOOT/BOOTX64.EFI";
/// Slot B's UKI on the ESP.
pub const SLOT_B_PATH: &str = "EFI/QSVS/B.EFI";

/// The ESP's FAT volume label.
pub const ESP_LABEL: &str = "QSVS ESP";
/// The ESP's FAT volume id: `QSVS`.
pub const ESP_VOLUME_ID: u32 = 0x5153_5653;

/// Bytes of the ESP: room for both slots' UKIs several times over.
pub const ESP_BYTES: u64 = 256 * MIB;
/// Bytes of the A/B record partition.
pub const RECORD_BYTES: u64 = MIB;
/// The least `QSVSDATA` may be.
pub const DATA_MIN_BYTES: u64 = 300 * MIB;
/// The size of a built disk image when none is asked for.
pub const DEFAULT_DISK_BYTES: u64 = 1024 * MIB;

/// The partitions in disk order.
pub fn parts() -> Vec<PartDesc> {
    vec![
        PartDesc {
            name: ESP_NAME.into(),
            type_guid: ESP_TYPE_GUID,
            size: Size::Fixed(ESP_BYTES),
            content: Content::Image,
        },
        PartDesc {
            name: RECORD_NAME.into(),
            type_guid: RECORD_TYPE_GUID,
            size: Size::Fixed(RECORD_BYTES),
            content: Content::Wiped,
        },
        PartDesc {
            name: DATA_NAME.into(),
            type_guid: DATA_TYPE_GUID,
            size: Size::Rest {
                min: DATA_MIN_BYTES,
            },
            content: Content::Xfs {
                label: DATA_NAME.into(),
                uuid: DATA_FS_UUID,
            },
        },
    ]
}

/// The partitions as an installer writes them: the data partition's
/// head is cleared and its file system made afterwards, on the
/// partition or on the mirrored set over two of them.
pub fn install_parts() -> Vec<PartDesc> {
    let mut parts = parts();
    if let Some(data) = parts.last_mut() {
        data.content = Content::Wiped;
    }
    parts
}

/// The smallest disk that can be a load source.
pub fn min_disk_bytes() -> u64 {
    diskimg::layout::min_bytes(&parts())
}

/// The disk and partition GUIDs of a built image: fixed, so the image
/// is the same bytes every build.
pub fn fixed_guids() -> LayoutGuids {
    LayoutGuids::fixed(*b"QSVS", parts().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diskimg::layout::Layout;

    #[test]
    fn the_default_disk_holds_the_three_partitions() {
        let layout = Layout::for_bytes(&parts(), DEFAULT_DISK_BYTES).unwrap();
        let names: Vec<&str> = layout.parts.iter().map(|p| p.desc.name.as_str()).collect();
        assert_eq!(names, [ESP_NAME, RECORD_NAME, DATA_NAME]);
        assert_eq!(layout.parts[0].part.bytes(), ESP_BYTES);
        assert_eq!(layout.parts[1].part.bytes(), RECORD_BYTES);
        assert!(layout.parts[2].part.bytes() >= DATA_MIN_BYTES);
    }

    #[test]
    fn the_types_and_guids_are_our_own() {
        assert_ne!(RECORD_TYPE_GUID, DATA_TYPE_GUID);
        for ours in [RECORD_TYPE_GUID, DATA_TYPE_GUID] {
            for theirs in [
                diskimg::LIC_TYPE_GUID,
                diskimg::ASP1_TYPE_GUID,
                diskimg::IFS_TYPE_GUID,
                ESP_TYPE_GUID,
            ] {
                assert_ne!(ours, theirs);
            }
        }
        let g = fixed_guids();
        assert_eq!(g.parts.len(), 3);
        assert_eq!(&g.disk.as_bytes()[..4], b"QSVS");
        assert_ne!(ESP_VOLUME_ID, diskimg::fat::VOLUME_ID);
    }
}
