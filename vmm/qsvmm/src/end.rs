// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Why a partition ended. `qsvmm` says so in its exit code; whoever
//! started it reads the code, or the signal that killed it, and posts
//! the reference code.
//!
//! | Exit | Why | Reference code |
//! |---|---|---|
//! | 0 | the guest powered off | `C2005100` |
//! | 10 | the guest asked for a restart | `C2005110` |
//! | 20 | the guest faulted | `A2005120` |
//! | 30 | the partition did not IPL | `A2005100` |
//! | 40 | the VMM failed | `A2005130` |
//! | 50 | the VMM was asked to end (`SIGTERM`) | `A2005140` |
//! | a signal | the VMM was killed | `A2005150` |
//!
//! A guest restart ends the process: it is not restarted in place.

use refcode::RefCode;

/// Why a partition ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The guest powered off.
    PoweredOff,
    /// The guest asked for a restart.
    Restart,
    /// The guest faulted: a triple fault, or KVM could not enter it.
    GuestFault,
    /// The partition did not IPL: its profile, its disks or its
    /// load source's contents.
    NotIpled,
    /// The VMM failed.
    VmmError,
    /// The VMM was asked to end.
    Ended,
    /// The VMM was killed by a signal.
    Killed,
}

impl End {
    /// The exit code of the process; none when a signal ended it.
    pub fn exit_code(self) -> Option<u8> {
        match self {
            End::PoweredOff => Some(0),
            End::Restart => Some(10),
            End::GuestFault => Some(20),
            End::NotIpled => Some(30),
            End::VmmError => Some(40),
            End::Ended => Some(50),
            End::Killed => None,
        }
    }

    /// What an exit code means. A code `qsvmm` does not use is a VMM
    /// error: a panic's 101, say.
    pub fn from_exit(code: Option<i32>) -> Self {
        match code {
            Some(0) => End::PoweredOff,
            Some(10) => End::Restart,
            Some(20) => End::GuestFault,
            Some(30) => End::NotIpled,
            Some(50) => End::Ended,
            Some(_) => End::VmmError,
            None => End::Killed,
        }
    }

    /// The reference code of the end.
    pub fn code(self) -> &'static RefCode {
        match self {
            End::PoweredOff => &refcode::PARTITION_POWERED_OFF,
            End::Restart => &refcode::PARTITION_RESTART,
            End::GuestFault => &refcode::PARTITION_GUEST_FAULT,
            End::NotIpled => &refcode::PARTITION_NOT_IPLED,
            End::VmmError => &refcode::PARTITION_VMM_ERROR,
            End::Ended => &refcode::PARTITION_ENDED,
            End::Killed => &refcode::PARTITION_KILLED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_end_has_its_own_exit_code_and_reference_code() {
        let all = [
            End::PoweredOff,
            End::Restart,
            End::GuestFault,
            End::NotIpled,
            End::VmmError,
            End::Ended,
            End::Killed,
        ];
        for end in all {
            assert_eq!(End::from_exit(end.exit_code().map(i32::from)), end);
        }
        let mut codes: Vec<&str> = all.iter().map(|e| e.code().code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
        // A panic, or any code that is not one of these.
        assert_eq!(End::from_exit(Some(101)), End::VmmError);
        assert_eq!(End::from_exit(Some(1)), End::VmmError);
    }
}
