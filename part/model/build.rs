// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `QSVS_TEST_SCHEMA=1` builds a test release whose partition
//! definitions are of schema version 2, for the update suites of
//! `docs/PHASE11.md` (Section 6): a release that changes a state file's
//! schema. A release image is never built with it: the userland step
//! clears it.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(test_schema)");
    println!("cargo::rerun-if-env-changed=QSVS_TEST_SCHEMA");
    if std::env::var("QSVS_TEST_SCHEMA").is_ok_and(|v| v == "1") {
        println!("cargo::rustc-cfg=test_schema");
    }
}
