// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Host-side steps that are QS/VS's own: the doctor rows, the neutrality
//! check of `base/`, the fixture that takes the base crates from outside,
//! QS/VS's fuzzer and the QEMU suites. The license header check, the
//! wording audit and the fuzzer's run are `base/buildaid`'s.

/// QS/VS's fuzzer (`tools/fuzz`).
pub const FUZZER: buildaid::fuzz::Fuzzer = buildaid::fuzz::Fuzzer {
    package: "fuzz",
    binary: "qsvsfuzz",
    corpus: "tools/fuzz/corpus",
};

/// `build fuzz`: QS/VS's fuzzer, every target or the one named.
pub fn fuzz(
    ctx: &buildkit::Ctx,
    only: Option<&str>,
    secs: u64,
    jobs: usize,
    save: bool,
) -> anyhow::Result<()> {
    buildaid::fuzz::fuzz(ctx, &FUZZER, only, secs, jobs, save)
}

/// The crates whose panels the help audit reads.
pub const PANELS: &[&str] = &["base/panels/src", "base/install/src", "svc/tools/src"];

pub mod catalogue;
pub mod doctor;
pub mod fixture;
pub mod neutral;
pub mod testrun;
