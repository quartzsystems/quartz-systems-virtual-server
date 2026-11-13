// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service table: fixed, compiled in. Starting follows `order`;
//! stopping goes the other way, so the log service is the last to end.

use std::time::Duration;

use supervise::Service;

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
};

/// The log service.
const LOGD: Service = Service {
    name: "logd",
    binary: "/qsvs/bin/qsvslogd",
    args: &[],
    order: 10,
    needs: &[],
    ready_within: READY_WITHIN,
};

/// The management network and time.
const NETD: Service = Service {
    name: "netd",
    binary: "/qsvs/bin/qsvsnetd",
    args: &[],
    order: 20,
    needs: &[],
    ready_within: READY_WITHIN,
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
};

/// The Service Tools console, on the display and the serial port.
pub const TOOLS: Service = Service {
    name: "tools",
    binary: "/qsvs/bin/qsvstools",
    args: &[],
    order: 30,
    needs: &[],
    ready_within: READY_WITHIN,
};

/// The base services.
#[cfg(not(test_hooks))]
pub static TABLE: &[Service] = &[STGD, LOGD, NETD, PARTD, TOOLS];

/// The base services and, in a test image, the test hooks' two.
#[cfg(test_hooks)]
pub static TABLE: &[Service] = &[
    STGD,
    LOGD,
    NETD,
    PARTD,
    TOOLS,
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
        for service in TABLE {
            assert!(service.binary.starts_with(release::BIN_DIR));
        }
        // No service is the VMM: the partition manager starts one
        // for each partition it activates, and nothing else does.
        assert!(TABLE.iter().all(|s| !s.binary.ends_with("qsvmm")));
        assert_eq!(TABLE[order[3]].name, "partd");
        assert!(
            TABLE
                .iter()
                .all(|s| s.name != "partition" || cfg!(test_hooks))
        );
    }
}
