// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What an appliance's load source carries that is the appliance's
//! own: the GPT types and names of its partitions, the vendor directory
//! of its ESP, the magics of its records. Two appliances so never take
//! one another's disk for their own.

/// The names, types and magics of one appliance's load source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadSourceNames {
    /// GPT type of the record partition, as the GUID's 128 bits.
    pub record_type: u128,
    /// GPT type of the data partition.
    pub data_type: u128,
    /// The xfs UUID of a built image's data file system.
    pub data_fs_uuid: u128,
    /// GPT name of the ESP.
    pub esp_name: &'static str,
    /// GPT name of the record partition.
    pub record_name: &'static str,
    /// GPT name and xfs label of the data partition.
    pub data_name: &'static str,
    /// The ESP's vendor directory, the payload's directory on the
    /// installation media and the first word of its manifest.
    pub vendor: &'static str,
    /// Slot B's UKI on the ESP, in the vendor directory.
    pub slot_b_path: &'static str,
    /// The ESP's FAT volume label.
    pub esp_label: &'static str,
    /// The ESP's FAT volume id.
    pub esp_volume_id: u32,
    /// What the fixed GUIDs of a built image start with.
    pub guid_seed: [u8; 4],
    /// The magic of the A/B record.
    pub record_magic: [u8; 8],
    /// The magic of a mirrored set's label.
    pub label_magic: [u8; 8],
    /// The magic of the resume map's record.
    pub resume_magic: [u8; 8],
    /// The magic the protection engine encodes its resume map under.
    pub resume_map_magic: [u8; 8],
}

impl LoadSourceNames {
    /// The names of no product: what the base crates' tests use.
    pub const DEMO: LoadSourceNames = LoadSourceNames {
        record_type: 0x0d5e_c0de_0000_4000_8000_0000_0000_0001,
        data_type: 0x0d5e_c0de_0000_4000_8000_0000_0000_0002,
        data_fs_uuid: 0x0d5e_c0de_0000_4000_8000_0000_0000_0003,
        esp_name: "QSDEMO ESP",
        record_name: "QSDEMO-AB",
        data_name: "QSDEMODATA",
        vendor: "QSDEMO",
        slot_b_path: "EFI/QSDEMO/B.EFI",
        esp_label: "QSDEMO ESP",
        esp_volume_id: 0x5153_444d,
        guid_seed: *b"QSDM",
        record_magic: *b"QSDMREC\0",
        label_magic: *b"QSDMSET\0",
        resume_magic: *b"QSDMRSM\0",
        resume_map_magic: *b"QSDMRMAP",
    };
}
