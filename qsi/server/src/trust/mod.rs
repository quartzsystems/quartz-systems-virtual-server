// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Who may connect: the consoles paired with this host, kept in a state
//! file, and the one pairing password out at a time. QS/400's trust
//! crate keeps nothing; this is what it asks its caller to keep
//! (`qsitrust::Pins`, `qsitrust::pair::Guard`).

mod consoles;
mod pairing;

pub use consoles::{Console, Consoles, MAX_CONSOLES};
pub use pairing::{ATTEMPTS, Happened, LIFETIME_SECS, PairingGuard, Password, Refusal, Trust};
