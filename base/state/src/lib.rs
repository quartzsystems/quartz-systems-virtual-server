// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An appliance's state: typed TOML files under `<data>/state/`, one
//! per concern, each with a `schema_version`.
//!
//! - [`store`]: reading (a newer version is refused, an older one is
//!   migrated step by step) and writing (atomically: a reader sees the
//!   old file or the new one, never part of either).
//! - [`doc`]: the files: `host.toml`, `network.toml`, `identity.toml`,
//!   `inventory.toml`.

#![forbid(unsafe_code)]

pub mod doc;
pub mod store;

pub use doc::host::Host;
pub use doc::identity::Identity;
pub use doc::inventory::Inventory;
pub use doc::network::{Cidr, Device, Mode, Network};
pub use store::{Document, Migration, StateError, Store, atomic_write};
