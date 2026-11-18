// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service table: fixed, compiled in. Starting follows `order`;
//! stopping goes the other way, so the log service is the last to end.

use std::time::Duration;

use supervise::Service;
use supervise::table::Caps;
#[allow(clippy::wildcard_imports)]
use supervise::table::caps::*;

/// What a base service is given to say it is ready.
const READY_WITHIN: Duration = Duration::from_secs(10);

/// The storage service: the load source, served before `/data` is
/// mounted and until it is unmounted.
pub const STGD: Service = Service {
    name: stgd::NAME,
    binary: "/qsvs/bin/qsvsstgd",
    args: &[],
    order: 5,
    needs: &[],
    ready_within: Duration::from_secs(30),
    caps: Caps::Only(&[
        SYS_ADMIN,
        SYS_RAWIO,
        IPC_LOCK,
        SYS_RESOURCE,
        DAC_OVERRIDE,
        FOWNER,
    ]),
};

/// The log service.
const LOGD: Service = Service {
    name: "logd",
    binary: "/qsvs/bin/qsvslogd",
    args: &[],
    order: 10,
    needs: &[],
    ready_within: READY_WITHIN,
    caps: Caps::Only(&[SYSLOG]),
};

/// The problem service: after the log service, whose history it
/// reads, so problems are recorded whatever else runs.
pub const PROBD: Service = Service {
    name: problem::proto::NAME,
    binary: "/qsvs/bin/qsvsprobd",
    args: &[],
    order: 12,
    needs: &[],
    ready_within: READY_WITHIN,
    caps: Caps::Only(&[CHOWN]),
};

/// The management network and time.
const NETD: Service = Service {
    name: "netd",
    binary: "/qsvs/bin/qsvsnetd",
    args: &[],
    order: 20,
    needs: &[],
    ready_within: READY_WITHIN,
    caps: Caps::Only(&[NET_ADMIN, NET_RAW, NET_BIND_SERVICE, SYS_TIME]),
};

/// The pool service: the storage pools, their virtual disks and the
/// media library. Before the partition manager, which asks it for a
/// partition's disks; after it when stopping, so every partition has
/// ended before a pool is stopped.
pub const POOLD: Service = Service {
    name: "poold",
    binary: "/qsvs/bin/qsvspoold",
    args: &[],
    order: 22,
    needs: &[],
    // Assembling a pool reads its labels and may repair a parity set.
    ready_within: Duration::from_secs(120),
    caps: Caps::Only(&[
        SYS_ADMIN,
        SYS_RAWIO,
        IPC_LOCK,
        SYS_RESOURCE,
        DAC_OVERRIDE,
        FOWNER,
        CHOWN,
    ]),
};

/// The switch service: the virtual switches and the partitions' ports.
/// After the network service, which keeps the management interface it
/// never takes; before the partition manager, which asks it for a
/// partition's ports; after it when stopping, and its kernel objects
/// are left as they are.
pub const VSWITCHD: Service = Service {
    name: vswitchd::NAME,
    binary: "/qsvs/bin/qsvsvswitchd",
    args: &[],
    order: 23,
    needs: &[],
    ready_within: Duration::from_secs(30),
    caps: Caps::Only(&[NET_ADMIN, NET_RAW, CHOWN]),
};

/// The partition manager: it owns every partition's VMM. Its
/// partitions go on when it is started again.
pub const PARTD: Service = Service {
    name: partd::proto::NAME,
    binary: "/qsvs/bin/qsvspartd",
    args: &[],
    order: 25,
    needs: &[],
    ready_within: Duration::from_secs(30),
    caps: Caps::Only(&[
        CHOWN,
        DAC_OVERRIDE,
        FOWNER,
        KILL,
        SETGID,
        SETUID,
        SETPCAP,
        IPC_LOCK,
        SYS_RAWIO,
        SYS_ADMIN,
        SYS_NICE,
        SYS_RESOURCE,
    ]),
};

/// The Service Tools console, on the display and the serial port.
pub const TOOLS: Service = Service {
    name: "tools",
    binary: "/qsvs/bin/qsvstools",
    args: &[],
    order: 30,
    needs: &[],
    ready_within: READY_WITHIN,
    caps: Caps::Only(&[SYS_TTY_CONFIG]),
};

/// The update service: staging, installing and activating code
/// updates. After the partition manager, which it asks to shut
/// partitions down; before the QSI server, which asks it.
pub const UPDD: Service = Service {
    name: update::proto::NAME,
    binary: "/qsvs/bin/qsvsupdd",
    args: &[],
    order: 32,
    needs: &[],
    ready_within: READY_WITHIN,
    caps: Caps::Only(&[CHOWN, SYS_ADMIN, SYS_RAWIO]),
};

/// The QSI host server: the one listener, last, so every service it
/// translates to is there; the first stopped.
pub const QSID: Service = Service {
    name: "qsid",
    binary: "/qsvs/bin/qsvsqsid",
    args: &[],
    order: 35,
    needs: &[],
    ready_within: READY_WITHIN,
    caps: Caps::Only(&[SETUID, SETGID, SETPCAP, CHOWN, FOWNER]),
};

/// The base services.
#[cfg(not(test_hooks))]
pub static TABLE: &[Service] = &[
    STGD, LOGD, PROBD, NETD, POOLD, VSWITCHD, PARTD, TOOLS, UPDD, QSID,
];

/// The base services and, in a test image, the test hooks' two.
#[cfg(test_hooks)]
pub static TABLE: &[Service] = &[
    STGD,
    LOGD,
    PROBD,
    NETD,
    POOLD,
    VSWITCHD,
    PARTD,
    TOOLS,
    UPDD,
    QSID,
    crate::hooks::FLAKY,
    crate::hooks::CHANNEL,
    crate::hooks::PARTITION,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_one_and_storage_then_the_log_service_start_first() {
        assert_eq!(supervise::table::check(TABLE), Ok(()));
        let order = supervise::table::start_order(TABLE);
        assert_eq!(TABLE[order[0]].name, "stgd");
        assert_eq!(TABLE[order[1]].name, "logd");
        assert_eq!(TABLE[order[2]].name, "probd");
        for service in TABLE {
            assert!(service.binary.starts_with(release::BIN_DIR));
        }
        // No service is the VMM: the partition manager starts one
        // for each partition it activates, and nothing else does.
        assert!(TABLE.iter().all(|s| !s.binary.ends_with("qsvmm")));
        // The pool service before the partition manager, which asks it
        // for a partition's disks.
        assert_eq!(TABLE[order[4]].name, "poold");
        // The switch service after the network service and before the
        // partition manager, which asks it for a partition's ports.
        assert_eq!(TABLE[order[3]].name, "netd");
        assert_eq!(TABLE[order[5]].name, "vswitchd");
        assert_eq!(TABLE[order[6]].name, "partd");
        assert!(
            TABLE
                .iter()
                .all(|s| s.name != "partition" || cfg!(test_hooks))
        );
    }
}
