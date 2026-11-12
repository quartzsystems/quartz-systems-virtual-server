// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service table: fixed, compiled in. Starting follows `order`;
//! stopping goes the other way, so the log service is the last to end.

use std::time::Duration;

use supervise::Service;

/// What a base service is given to say it is ready.
const READY_WITHIN: Duration = Duration::from_secs(10);

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

/// The base services.
#[cfg(not(test_hooks))]
pub static TABLE: &[Service] = &[LOGD, NETD];

/// The base services and, in a test image, the test hooks' two.
#[cfg(test_hooks)]
pub static TABLE: &[Service] = &[LOGD, NETD, crate::hooks::FLAKY, crate::hooks::CHANNEL];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_one_and_the_log_service_starts_first() {
        assert_eq!(supervise::table::check(TABLE), Ok(()));
        let order = supervise::table::start_order(TABLE);
        assert_eq!(TABLE[order[0]].name, "logd");
        for service in TABLE {
            assert!(service.binary.starts_with(release::BIN_DIR));
        }
    }
}
