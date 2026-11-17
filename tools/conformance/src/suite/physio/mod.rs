// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Phase 10's suites: physical I/O and tape pass-through, on the test
//! image.
//!
//! - [`tape`]: Section 1, QS/400 saving to and restoring from the
//!   virtual tape drive.
//! - [`devices`]: a host's devices given to partitions.
//! - [`tools`]: a release at Service Tools option 10.
//! - [`drive`]: tape pass-through against the double.
//! - [`isolation`]: the IOMMU's mappings of a device given away.
//! - [`hostile`]: a guest that writes everything at its device.
//! - [`entitle`]: a running shared partition's entitlement changed.

pub mod devices;
mod drive;
mod entitle;
mod host;
mod hostile;
mod isolation;
mod tape;
mod tools;

use super::base::SuiteFn;

/// The suites, in the order of Phase 10's handoff: its first, every
/// earlier suite on the new pin, is the build tool's.
pub const SUITES: &[(&str, SuiteFn)] = &[
    ("pio-tape-save", tape::save),
    ("pio-tape-span", tape::span),
    ("pio-tape-volumes", tape::volumes),
    ("pio-tools", tools::service_tools),
    ("pio-nvme", devices::nvme),
    ("pio-refused", host::refused),
    ("pio-group", host::group),
    ("pio-iommu-off", host::iommu_off),
    ("pio-held", host::held),
    ("pio-reset", host::reset),
    ("pio-restart", host::restart),
    ("pio-isolation", isolation::isolation),
    ("pio-hostile", hostile::hostile),
    ("pio-tape-double", drive::double),
    ("pio-entitlement", entitle::entitlement),
];
