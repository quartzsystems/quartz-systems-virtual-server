// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The capabilities a service keeps. A service runs as root; before it
//! starts, every capability it does not keep is taken from its bounding
//! set and it is given no new privileges, so after `exec` it holds
//! exactly what it keeps, and so does everything it starts.

/// What a service keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caps {
    /// Every capability: a test image's hooks alone.
    All,
    /// These and no other.
    Only(&'static [u8]),
}

impl Caps {
    /// Whether `cap` is kept.
    pub fn keeps(self, cap: u8) -> bool {
        match self {
            Caps::All => true,
            Caps::Only(kept) => kept.contains(&cap),
        }
    }

    /// The bits of what is kept, as `/proc/<pid>/status` shows a set.
    pub fn mask(self) -> u64 {
        (0..64u8)
            .filter(|c| self.keeps(*c))
            .fold(0, |m, c| m | (1u64 << c))
    }
}

/// The capabilities by name (`linux/capability.h`).
pub const CHOWN: u8 = 0;
/// Pass over a file's mode.
pub const DAC_OVERRIDE: u8 = 1;
/// Act as the owner of any file.
pub const FOWNER: u8 = 3;
/// Signal any process.
pub const KILL: u8 = 5;
/// Change group IDs.
pub const SETGID: u8 = 6;
/// Change user IDs.
pub const SETUID: u8 = 7;
/// Take from the bounding set.
pub const SETPCAP: u8 = 8;
/// Bind ports below 1024.
pub const NET_BIND_SERVICE: u8 = 10;
/// Configure the network.
pub const NET_ADMIN: u8 = 12;
/// Raw and packet sockets.
pub const NET_RAW: u8 = 13;
/// Lock memory.
pub const IPC_LOCK: u8 = 14;
/// Raw I/O: SCSI pass-through of any command.
pub const SYS_RAWIO: u8 = 17;
/// Mounts, namespaces, block devices' controls.
pub const SYS_ADMIN: u8 = 21;
/// Priorities and processors of other processes.
pub const SYS_NICE: u8 = 23;
/// Past resource limits.
pub const SYS_RESOURCE: u8 = 24;
/// Set the clock.
pub const SYS_TIME: u8 = 25;
/// The console's and terminals' controls.
pub const SYS_TTY_CONFIG: u8 = 26;
/// Read the kernel's log.
pub const SYSLOG: u8 = 34;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_keeps_what_it_names_and_nothing_else() {
        let net = Caps::Only(&[NET_ADMIN, NET_RAW]);
        assert!(net.keeps(NET_ADMIN));
        assert!(!net.keeps(SYS_ADMIN));
        assert_eq!(net.mask(), (1 << 12) | (1 << 13));
        assert_eq!(Caps::Only(&[]).mask(), 0);
        assert_eq!(Caps::All.mask(), u64::MAX);
    }
}
