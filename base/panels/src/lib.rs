// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the installer and the service console of an appliance share,
//! over QS/400's `console` and screen kit. Nothing here names a
//! product: titles, the system name and every fact shown come from the
//! caller.
//!
//! - [`panel`]: a panel with help and a tick, and the stack of them.
//! - [`kit`]: small things every panel uses.
//! - [`forms`]: the host name and network, the time, a service tools
//!   user: what was typed, its screen and its check.
//! - [`screens`]: the console on the framebuffer and on a serial
//!   terminal, and the loop that runs a stack of panels on it (Linux).
//! - [`tools`]: the panels every service console has.
//! - [`keep`]: the one panel shown before the data file system exists,
//!   when the units of a mirrored load source ran apart.

#![forbid(unsafe_code)]

pub mod forms;
pub mod keep;
pub mod kit;
pub mod panel;
#[cfg(target_os = "linux")]
pub mod screens;
pub mod tools;

pub use panel::{Help, Named, Panel, PanelFlow, Runner};
pub use scrkit::panel::{End, Flow};
pub use scrkit::ui;
