// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The description an appliance hands the base crates. They are
//! product-neutral: every name, directory and reference code comes from
//! the [`Appliance`] their caller gives them.
//!
//! - [`Appliance`]: the identifier, the directories, the banner, how an
//!   [`Event`] is posted and how a fact reaches the status screen.
//! - [`time`]: civil dates from the UTC clock, and the display zone.
//! - [`links`]: the Ethernet links sysfs shows.
//! - [`names`]: what its load source carries that is its own.

#![forbid(unsafe_code)]

pub mod event;
pub mod links;
pub mod names;
pub mod time;

use std::path::PathBuf;

pub use event::{Event, Storage};
pub use names::LoadSourceNames;

/// One appliance, as the base crates see it.
#[derive(Clone, Copy, Debug)]
pub struct Appliance {
    /// Lower-case product identifier; PID 1 is `<id>init`.
    pub id: &'static str,
    /// The name as written.
    pub name: &'static str,
    /// The release banner.
    pub banner: fn() -> String,
    /// Where the data file system is mounted.
    pub data_dir: &'static str,
    /// The run directory (a tmpfs).
    pub run_dir: &'static str,
    /// What a default host name starts with.
    pub host_prefix: &'static str,
    /// The FAT volume label of a seed volume.
    pub seed_label: &'static str,
    /// What its load source carries that is its own.
    pub load_source: &'static LoadSourceNames,
    /// Post the reference code the product has for an event.
    pub post: fn(&Event<'_>),
    /// Tell PID 1 a fact for its status screen.
    pub note: fn(key: &str, value: &str),
}

impl Appliance {
    /// `<data>/state`: the state files.
    pub fn state_dir(&self) -> PathBuf {
        PathBuf::from(self.data_dir).join("state")
    }

    /// `<data>/log`: the logs.
    pub fn log_dir(&self) -> PathBuf {
        PathBuf::from(self.data_dir).join("log")
    }

    /// PID 1's control socket, `<run>/<id>init.sock`.
    pub fn control_socket(&self) -> PathBuf {
        PathBuf::from(self.run_dir).join(format!("{}init.sock", self.id))
    }

    /// The log service's record socket.
    pub fn log_socket(&self) -> PathBuf {
        PathBuf::from(self.run_dir).join("logd.sock")
    }

    /// The network service's request socket.
    pub fn net_socket(&self) -> PathBuf {
        PathBuf::from(self.run_dir).join("netd.sock")
    }

    /// A file of the storage service in the run directory,
    /// `<run>/<id>stgd.<what>`.
    pub fn storage_file(&self, what: &str) -> PathBuf {
        PathBuf::from(self.run_dir).join(format!("{}stgd.{what}", self.id))
    }
}

/// The key of the status screen's management line.
pub const NOTE_MANAGEMENT: &str = "management";

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Appliance {
        Appliance {
            load_source: &LoadSourceNames::DEMO,
            id: "qsdemo",
            name: "QSDEMO",
            banner: || "QSDEMO V0R1M0 test".into(),
            data_dir: "/data",
            run_dir: "/run",
            host_prefix: "QSDEMO",
            seed_label: "QSDEMOSEED",
            post: |_| {},
            note: |_, _| {},
        }
    }

    #[test]
    fn the_paths_come_from_the_description() {
        let a = demo();
        assert_eq!(a.state_dir(), PathBuf::from("/data/state"));
        assert_eq!(a.log_dir(), PathBuf::from("/data/log"));
        assert_eq!(a.control_socket(), PathBuf::from("/run/qsdemoinit.sock"));
        assert_eq!(a.log_socket(), PathBuf::from("/run/logd.sock"));
        assert_eq!(a.net_socket(), PathBuf::from("/run/netd.sock"));
        assert_eq!(
            a.storage_file("sock"),
            PathBuf::from("/run/qsdemostgd.sock")
        );
    }
}
