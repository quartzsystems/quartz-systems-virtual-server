// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The boot disk's partitions on a running host, found by GPT type
//! through sysfs (QS/400's `diskimg::locate`).

use std::path::PathBuf;

use diskimg::efi::EspLocation;

use crate::layout::{DATA_TYPE_GUID, RECORD_TYPE_GUID};

/// `/dev/<partition>` of `QSVSDATA`.
pub fn data() -> Result<PathBuf, String> {
    diskimg::locate::first_of_type(DATA_TYPE_GUID).map_err(|e| e.to_string())
}

/// The ESP on the disk that holds the A/B record: its node, and where
/// it is for a firmware boot entry.
pub fn esp() -> Result<(PathBuf, EspLocation), String> {
    let record = diskimg::locate::first_of_type(RECORD_TYPE_GUID).ok();
    diskimg::locate::esp(record.as_deref()).map_err(|e| e.to_string())
}
