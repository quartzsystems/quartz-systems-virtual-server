// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The processor check of `base/processor`, in QS/VS's words: the
//! server, PID 1 and Service Tools ask here.

pub use processor::{NEEDED, lacking, lacking_from};

/// What a person is told when something is lacking.
#[must_use]
pub fn problem(lacking: &[&str]) -> String {
    format!(
        "The processor lacks {}: the service interface is down.",
        lacking.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_problem_names_what_is_lacking() {
        assert_eq!(lacking_from("avx2 adx"), ["AES", "PCLMULQDQ"]);
        assert_eq!(
            problem(&["AVX2", "ADX"]),
            "The processor lacks AVX2, ADX: the service interface is down."
        );
    }
}
