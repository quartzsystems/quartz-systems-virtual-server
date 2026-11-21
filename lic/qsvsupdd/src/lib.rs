// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What is QS/VS's in a code update (`docs/UPDATE.md`), over
//! `base/update`: the image's format, the messages it refuses with, the
//! codes it posts, the state files a release reads, and the partitions
//! shut down before the restart.
//!
//! - [`schemas`]: the state files this release reads, and their versions.

#![forbid(unsafe_code)]

pub mod schemas;

use update::{Code, Format, Messages};

/// A QS/VS update image: `QSVSUPDT`, `QSVS-UPDATE 1`, fixes `QVnnnnn`.
pub static FORMAT: Format = Format {
    magic: *b"QSVSUPDT",
    head: "QSVS-UPDATE 1",
    fix_prefix: "QV",
};

/// The catalogue's messages for each refusal.
pub static MESSAGES: Messages = Messages {
    busy: &qvsmsg::UPDATE_BUSY,
    cut: &qvsmsg::UPDATE_CUT,
    damaged: &qvsmsg::UPDATE_DAMAGED,
    name: &qvsmsg::UPDATE_NAME,
    not_kept: &qvsmsg::UPDATE_NOT_KEPT,
    not_verified: &qvsmsg::UPDATE_NOT_VERIFIED,
    not_written: &qvsmsg::UPDATE_NOT_WRITTEN,
    nothing_pending: &qvsmsg::UPDATE_NOTHING_PENDING,
    nothing_staged: &qvsmsg::UPDATE_NOTHING_STAGED,
    older: &qvsmsg::UPDATE_OLDER,
    partitions_run: &qvsmsg::UPDATE_PARTITIONS_RUN,
    pending: &qvsmsg::UPDATE_PENDING,
    product: &qvsmsg::UPDATE_PRODUCT,
    record: &qvsmsg::UPDATE_RECORD,
    signature: &qvsmsg::UPDATE_SIGNATURE,
    staged_already: &qvsmsg::UPDATE_STAGED_ALREADY,
    state: &qvsmsg::UPDATE_STATE,
};

/// The host's reference code for one of the service's.
#[must_use]
pub fn code(c: Code) -> &'static refcode::RefCode {
    match c {
        Code::Staged => &refcode::UPDATE_STAGED,
        Code::Verified => &refcode::UPDATE_VERIFIED,
        Code::Refused => &refcode::UPDATE_REFUSED,
        Code::Discarded => &refcode::UPDATE_DISCARDED,
        Code::Installing => &refcode::UPDATE_INSTALLING,
        Code::SlotWritten => &refcode::UPDATE_SLOT_WRITTEN,
        Code::Installed => &refcode::UPDATE_INSTALLED,
        Code::InstallFailed => &refcode::UPDATE_INSTALL_FAILED,
        Code::Cancelled => &refcode::UPDATE_CANCELLED,
    }
}

/// The host's partitions, by the partition manager at its socket.
#[cfg(target_os = "linux")]
pub struct Partd(pub partd::Client);

#[cfg(target_os = "linux")]
impl update::Partitions for Partd {
    fn running(&mut self) -> Vec<(u32, String)> {
        use partd::Calls as _;
        self.0
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.state.active())
            .map(|p| (p.id, p.name))
            .collect()
    }

    /// Shut each down through its system, then end at once each still
    /// active after `timeout` seconds.
    fn end(&mut self, ids: &[u32], timeout: u32) {
        use partd::Calls as _;
        use std::time::{Duration, Instant};
        let partd = &mut self.0;
        for id in ids {
            let _ = partd.shut_down_as(*id, partd::proto::Shutdown::System, timeout, false);
        }
        let active = |p: &mut partd::Client| -> Vec<u32> {
            p.list()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.state.active() && ids.contains(&s.id))
                .map(|s| s.id)
                .collect()
        };
        let until = Instant::now() + Duration::from_secs(u64::from(timeout));
        while Instant::now() < until && !active(partd).is_empty() {
            std::thread::sleep(Duration::from_millis(500));
        }
        for id in active(partd) {
            let _ = partd.shut_down(id);
        }
        let until = Instant::now() + Duration::from_secs(60);
        while Instant::now() < until && !active(partd).is_empty() {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_qsvs_image_is_as_it_always_was() {
        assert_eq!(&super::FORMAT.magic, b"QSVSUPDT");
        assert_eq!(super::FORMAT.head, "QSVS-UPDATE 1");
        assert!(update::manifest::valid_fix(&super::FORMAT, "QV00001"));
        assert!(!update::manifest::valid_fix(&super::FORMAT, "QV1"));
    }
}
