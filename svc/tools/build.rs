// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `QSVS_TEST_HOOKS=1` builds the Service Tools of a test image: its
//! one test hook is compiled in under `cfg(test_hooks)` and in no
//! other build.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(test_hooks)");
    println!("cargo::rerun-if-env-changed=QSVS_TEST_HOOKS");
    if std::env::var("QSVS_TEST_HOOKS").is_ok_and(|v| v == "1") {
        println!("cargo::rustc-cfg=test_hooks");
    }
}
