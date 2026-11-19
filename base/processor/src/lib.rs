// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What QSI's cryptography needs of the processor (QS/400's
//! `docs/QSI.md` section 12): graviola uses AVX2, ADX, AES and
//! PCLMULQDQ, and a processor without one of them faults at its first
//! use. An appliance asks here before anything touches it, and says
//! in its own words what is lacking.

#![forbid(unsafe_code)]

/// Each feature needed: its `/proc/cpuinfo` flag and its name.
pub const NEEDED: [(&str, &str); 4] = [
    ("avx2", "AVX2"),
    ("adx", "ADX"),
    ("aes", "AES"),
    ("pclmulqdq", "PCLMULQDQ"),
];

/// The names of what is needed and not among `flags` (a `flags` line
/// of `/proc/cpuinfo`, or any words).
#[must_use]
pub fn lacking_from(flags: &str) -> Vec<&'static str> {
    NEEDED
        .iter()
        .filter(|(flag, _)| !flags.split_ascii_whitespace().any(|f| f == *flag))
        .map(|(_, name)| *name)
        .collect()
}

/// The names of what this processor lacks; none when it has all.
#[must_use]
pub fn lacking() -> Vec<&'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        let has = [
            std::is_x86_feature_detected!("avx2"),
            std::is_x86_feature_detected!("adx"),
            std::is_x86_feature_detected!("aes"),
            std::is_x86_feature_detected!("pclmulqdq"),
        ];
        NEEDED
            .iter()
            .zip(has)
            .filter(|(_, has)| !has)
            .map(|((_, name), _)| *name)
            .collect()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        NEEDED.iter().map(|(_, name)| *name).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_lacking_is_named() {
        assert!(lacking_from("fpu sse2 avx2 adx aes pclmulqdq bmi2").is_empty());
        assert_eq!(lacking_from("fpu sse2 aes pclmulqdq"), ["AVX2", "ADX"]);
        assert_eq!(lacking_from(""), ["AVX2", "ADX", "AES", "PCLMULQDQ"]);
    }

    #[test]
    fn the_names_are_the_detected_features() {
        // Each flag is one `is_x86_feature_detected!` knows by that name.
        assert_eq!(
            NEEDED.map(|(flag, _)| flag),
            ["avx2", "adx", "aes", "pclmulqdq"]
        );
    }
}
