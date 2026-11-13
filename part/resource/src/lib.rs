// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A host's resources for its partitions (`docs/RESOURCES.md`):
//! processors and memory, counted, given out at activation and set in
//! the kernel's controls. All of it is arithmetic over what it is
//! given: nothing here reads the machine or writes a file.
//!
//! - [`topology`]: cores and their threads.
//! - [`settings`]: `partitions.toml`: the reserve, the block size, the
//!   limit, the shared processor pools.
//! - [`account`]: installed, configurable and available; what an
//!   activation gets.
//! - [`cgroup`]: the cgroup tree as a plan.

#![forbid(unsafe_code)]

pub mod account;
pub mod cgroup;
pub mod settings;
pub mod topology;

pub use account::{Amount, Current, Held, Host, HostResources, PoolStatus, Processors, Vcpus};
pub use settings::{Pool, Settings};
pub use topology::{Thread, Topology};
