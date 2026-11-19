// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Host-side steps that are QS/VS's own: the doctor rows, the license
//! header check, the neutrality check of `base/`, the fixture that takes
//! the base crates from outside, and the QEMU suites.

pub mod catalogue;
pub mod doctor;
pub mod fixture;
pub mod fuzz;
pub mod header;
pub mod neutral;
pub mod testrun;
pub mod wording;
