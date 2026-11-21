// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `Work with disk units`: `base/panels::units` with QS/VS's options
//! beside the load source's, the host's disk configuration, storage
//! pools, virtual disks and media library, and its help.

use panels::Flow;
use panels::units::{DiskUnits, Product};

use crate::env::Env;
use crate::menu::Outcome;

/// QS/VS's options and words on the load source's panel.
pub static PRODUCT: Product<Env, Outcome> = Product {
    options: &[
        (6, "Display disk configuration"),
        (7, "Work with storage pools"),
        (8, "Display virtual disks"),
        (9, "Display media library"),
    ],
    open: |n| {
        Some(match n {
            6 => Flow::Push(Box::new(crate::pools::DiskConfig)),
            7 => Flow::Push(Box::new(crate::pools::Pools)),
            8 => Flow::Push(Box::new(crate::pools::Vdisks)),
            9 => Flow::Push(Box::new(crate::pools::Media)),
            _ => return None,
        })
    },
    help: &[
        "The load source is the disk, or the mirrored pair of disks, the",
        "host starts from and keeps its own state on. Each unit is shown",
        "by its location code, with its state: Unprotected, Active,",
        "Resuming (being written from the other), Failed or Missing.",
        "",
        "1 adds a disk to a load source of one disk and mirrors it. The",
        "protection starts at the next IPL; restart the host from the",
        "main menu when it suits.",
        "",
        "2 puts another disk in the place of a unit that is missing or",
        "failed and rebuilds it while the host runs.",
        "",
        "6 shows every disk of the host and what it is. 7 works with the",
        "storage pools: made, watched, a unit replaced, hot spares, the",
        "media repository, an image imported. 8 and 9 show the virtual",
        "disks and the media library, which are managed from QS/MC.",
    ],
    others: " (option 6 shows each and what it is)",
    home: || Box::new(crate::menu::MainMenu),
};

/// The panel option 2 opens.
pub fn panel() -> DiskUnits<Env, Outcome> {
    DiskUnits::new(&PRODUCT)
}
