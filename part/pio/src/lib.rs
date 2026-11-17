// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Physical I/O (`docs/PHYSICALIO.md`): a host's PCI devices given to
//! partitions whole, each with the rest of its isolation group.
//!
//! - [`device`]: what the host uses, and each device as an operator and
//!   a console see it: released or not, why it cannot be, who holds it.
//! - [`released`]: `physio.toml`, the devices released for partitions.
//! - [`assign`]: what an activation is given of a profile's physical
//!   I/O, and every reason it is not.
//! - [`bind`]: the kernel's PCI files: a function given to the
//!   pass-through driver or back, reset, its group.
//!
//! But for [`bind`], this is arithmetic, as `resource` is: it reads no
//! file of the system and writes none. The partition manager and PID 1
//! bring it the facts.

#![forbid(unsafe_code)]

pub mod assign;
pub mod bind;
pub mod device;
pub mod released;

pub use assign::{Assignment, assignment, check_profile};
pub use device::{Blocker, Device, Holder, HostUses, Use, assess};
pub use released::{Released, ReleasedDevice};
