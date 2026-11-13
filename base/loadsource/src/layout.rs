// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partitions of a load-source disk and the fixed GUIDs of a built
//! image. Their types and names are the appliance's.

use appliance::LoadSourceNames;
use diskimg::layout::{Content, ESP_TYPE_GUID, LayoutGuids, PartDesc, Size};
use uuid::Uuid;

const MIB: u64 = 1024 * 1024;

/// Slot A's UKI on the ESP: the removable-media default boot path.
pub const SLOT_A_PATH: &str = "EFI/BOOT/BOOTX64.EFI";

/// Bytes of the ESP: room for both slots' UKIs several times over.
pub const ESP_BYTES: u64 = 256 * MIB;
/// Bytes of the record partition.
pub const RECORD_BYTES: u64 = MIB;
/// The least the data partition may be.
pub const DATA_MIN_BYTES: u64 = 300 * MIB;
/// The size of a built disk image when none is asked for.
pub const DEFAULT_DISK_BYTES: u64 = 1024 * MIB;

/// GPT type of the record partition.
pub fn record_type(names: &LoadSourceNames) -> Uuid {
    Uuid::from_u128(names.record_type)
}

/// GPT type of the data partition.
pub fn data_type(names: &LoadSourceNames) -> Uuid {
    Uuid::from_u128(names.data_type)
}

/// The partitions in disk order.
pub fn parts(names: &LoadSourceNames) -> Vec<PartDesc> {
    vec![
        PartDesc {
            name: names.esp_name.into(),
            type_guid: ESP_TYPE_GUID,
            size: Size::Fixed(ESP_BYTES),
            content: Content::Image,
        },
        PartDesc {
            name: names.record_name.into(),
            type_guid: record_type(names),
            size: Size::Fixed(RECORD_BYTES),
            content: Content::Wiped,
        },
        PartDesc {
            name: names.data_name.into(),
            type_guid: data_type(names),
            size: Size::Rest {
                min: DATA_MIN_BYTES,
            },
            content: Content::Xfs {
                label: names.data_name.into(),
                uuid: Uuid::from_u128(names.data_fs_uuid),
            },
        },
    ]
}

/// The partitions as an installer writes them: the data partition's
/// head is cleared and its file system made afterwards, on the
/// partition or on the mirrored set over two of them.
pub fn install_parts(names: &LoadSourceNames) -> Vec<PartDesc> {
    let mut parts = parts(names);
    if let Some(data) = parts.last_mut() {
        data.content = Content::Wiped;
    }
    parts
}

/// The smallest disk that can be a load source.
pub fn min_disk_bytes(names: &LoadSourceNames) -> u64 {
    diskimg::layout::min_bytes(&parts(names))
}

/// The disk and partition GUIDs of a built image: fixed, so the image
/// is the same bytes every build.
pub fn fixed_guids(names: &LoadSourceNames) -> LayoutGuids {
    LayoutGuids::fixed(names.guid_seed, parts(names).len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use diskimg::layout::Layout;

    const NAMES: &LoadSourceNames = &LoadSourceNames::DEMO;

    #[test]
    fn the_default_disk_holds_the_three_partitions() {
        let layout = Layout::for_bytes(&parts(NAMES), DEFAULT_DISK_BYTES).unwrap();
        let names: Vec<&str> = layout.parts.iter().map(|p| p.desc.name.as_str()).collect();
        assert_eq!(names, ["QSDEMO ESP", "QSDEMO-AB", "QSDEMODATA"]);
        assert_eq!(layout.parts[0].part.bytes(), ESP_BYTES);
        assert_eq!(layout.parts[1].part.bytes(), RECORD_BYTES);
        assert!(layout.parts[2].part.bytes() >= DATA_MIN_BYTES);
    }

    #[test]
    fn the_types_and_guids_are_the_appliance_s() {
        assert_ne!(record_type(NAMES), data_type(NAMES));
        assert_eq!(
            record_type(NAMES).to_string(),
            "0d5ec0de-0000-4000-8000-000000000001"
        );
        let g = fixed_guids(NAMES);
        assert_eq!(g.parts.len(), 3);
        assert_eq!(&g.disk.as_bytes()[..4], b"QSDM");
    }
}
