// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The installer of an appliance, as far as every appliance has the
//! same one: the screens in order, each field checked where it is
//! typed, and nothing written before the operator has seen the summary
//! and pressed F10. What is written, and how, is the product's: it
//! describes its disks and interfaces and carries out its steps
//! through [`Target`].
//!
//! - [`flow`]: the screens.
//! - [`answers`]: the answers of an unattended installation, from the
//!   `[install]` table of a seed file.

#![forbid(unsafe_code)]

pub mod answers;
pub mod flow;

use panels::forms::{LinkChoice, TimeChoice};
use state::Network;

/// One disk an operator may choose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskChoice {
    /// The kernel's name.
    pub name: String,
    /// The location code.
    pub location: String,
    /// The model.
    pub model: String,
    /// The serial number.
    pub serial: String,
    /// Its size.
    pub bytes: u64,
}

/// Everything an installation needs to be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answers {
    /// The load-source disks: one, or two for mirrored protection.
    pub disks: Vec<DiskChoice>,
    /// The host name.
    pub host: String,
    /// The management network.
    pub network: Network,
    /// The display zone, the time servers and the clock to set.
    pub time: TimeChoice,
    /// The service tools user ID.
    pub user_id: String,
    /// The hash of that user's password. The password is not kept.
    pub password_hash: String,
}

impl Answers {
    /// Whether the load source is mirrored.
    pub fn mirrored(&self) -> bool {
        self.disks.len() == 2
    }
}

/// One step of the installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// What the progress display calls it.
    pub name: &'static str,
    /// The reference code posted when it is done.
    pub code: &'static str,
}

/// How the installer ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The appliance is installed: restart into it.
    Installed,
    /// The operator left: restart without having written anything.
    Left,
}

/// The product, as its installer sees it.
pub trait Target {
    /// The product's name, `QS/VS`.
    fn product(&self) -> &'static str;
    /// The release banner.
    fn banner(&self) -> String;
    /// What the installation will do, a line each, for the welcome.
    fn what(&self) -> &'static [&'static str];
    /// The disks of the machine.
    fn disks(&mut self) -> Vec<DiskChoice>;
    /// The smallest disk that can be a load source.
    fn min_disk_bytes(&self) -> u64;
    /// The Ethernet links of the machine.
    fn links(&mut self) -> Vec<LinkChoice>;
    /// The host name used when the operator gives none.
    fn default_host(&self) -> String;
    /// The clock: seconds since the epoch.
    fn now(&self) -> i64;
    /// The steps, in order.
    fn steps(&self) -> &'static [Step];
    /// Carry out step `index` of [`Target::steps`] with `answers`.
    fn step(&mut self, index: usize, answers: &Answers) -> Result<(), String>;
    /// The reference code of a step that failed.
    fn failed(&self) -> &'static str;
    /// Say something for the installer's log; never a password.
    fn log(&mut self, line: &str);
}
