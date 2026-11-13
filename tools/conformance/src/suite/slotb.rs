// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The slot B boot: a copy of OVMF's variable store with a boot entry
//! for `EFI/QSVS/B.EFI` on the image's ESP, first in the boot order.
//! `buildkit`'s fixture boots no second slot; QS/400 chooses its slot
//! by firmware boot entry, and so does this.

use std::fs;
use std::path::{Path, PathBuf};

use diskimg::efi::{self, EspLocation};
use release::SLOT_B_PATH;

use crate::error::HarnessError;
use crate::qemu::{firmware, vars};

const STEP: &str = "IPL slot B";

fn setup(reason: &dyn std::fmt::Display) -> HarnessError {
    HarnessError::Setup {
        step: STEP,
        reason: reason.to_string(),
    }
}

/// Where the ESP is on the image, by its table: a built image has the
/// fixed GUIDs, an installed disk its own.
fn esp(image: &Path) -> Result<EspLocation, HarnessError> {
    crate::qemu::image::esp(image).map_err(|e| setup(&e))
}

/// Write the variable store under `work` and return its path.
pub fn vars(image: &Path, code: &Path, work: &Path) -> Result<PathBuf, HarnessError> {
    let template = fs::read(firmware::locate_vars(code)?)?;
    let option = efi::load_option("QS/VS slot B", &esp(image)?, SLOT_B_PATH);
    let store = vars::with_boot_entry(&template, &option).map_err(|e| setup(&e))?;
    let path = work.join("slot-b-vars.fd");
    fs::write(&path, store)?;
    Ok(path)
}
