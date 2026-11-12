// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's identity over QS/400's `vrm` crate: the release, the build
//! ID, the product text, the banner, and the prefixes everything named
//! after the product starts with.

#![forbid(unsafe_code)]

pub use vrm::Release;

/// The release `build.rs` captured: `VERSION` (`1.0.0` is `V1R0M0`), or
/// `QSVS_RELEASE`.
pub const RELEASE: Release = vrm::release_from_env!("QSVS_");

/// The build ID: `git describe`, or `QSVS_BUILD_ID`.
pub const BUILD_ID: &str = env!("QSVS_BUILD_ID");

/// The product's short name.
pub const NAME: &str = "QS/VS";

/// The product text.
pub const PRODUCT: &str = "Quartz Systems/Virtual Server";

/// Prefix of a fix identifier, `QVnnnnn`.
pub const FIX_PREFIX: &str = "QV";

/// Prefix of a message identifier.
pub const MSG_PREFIX: &str = "QVS";

/// Prefix of every environment variable QS/VS reads.
pub const ENV_PREFIX: &str = "QSVS_";

/// Prefix of QS/VS's kernel command-line keys.
pub const KEY_PREFIX: &str = "qsvs.";

/// The banner for a release and a build ID: `QS/VS V1R0M0 a1b2c3d`.
pub fn banner_of(release: Release, build_id: &str) -> String {
    format!("{NAME} {release} {build_id}")
}

/// This build's banner.
pub fn banner() -> String {
    banner_of(RELEASE, BUILD_ID)
}

/// A fix identifier, `QV00001`.
pub fn fix_id(number: u32) -> String {
    format!("{FIX_PREFIX}{number:05}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_names_the_release_and_the_build() {
        let r = Release { v: 1, r: 0, m: 0 };
        assert_eq!(banner_of(r, "abc1234"), "QS/VS V1R0M0 abc1234");
        assert!(banner().starts_with("QS/VS V"));
        assert!(banner().ends_with(BUILD_ID));
    }

    #[test]
    fn fixes_are_qv_and_five_digits() {
        assert_eq!(fix_id(1), "QV00001");
        assert_eq!(fix_id(12345), "QV12345");
    }
}
