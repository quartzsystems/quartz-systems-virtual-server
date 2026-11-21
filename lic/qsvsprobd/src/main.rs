// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's problem service (`docs/PROBLEMS.md`): `base/problem` run as
//! QS/VS, with the catalogue's table of the host's conditions. It reads
//! the reference code history the log service writes, keeps the problem
//! log in `/data/problems`, and shares its socket with the QSI server's
//! group.

/// The data file system nearly full (`A2001045`), or with room again
/// (`C2001045`).
#[cfg(target_os = "linux")]
fn post_data(full: bool, detail: &str, about: &[supervise::control::About]) {
    let code = if full {
        &refcode::DATA_NEARLY_FULL
    } else {
        &refcode::DATA_HAS_ROOM
    };
    refcode::post::remote::code_about(code, detail, about);
}

#[cfg(target_os = "linux")]
fn main() {
    use std::path::PathBuf;

    let appliance = release::appliance(|_| {}, |_, _| {});
    let host = problem::serve::Host {
        rules: qvsmsg::problems::RULES,
        post_data,
        full_code: refcode::DATA_NEARLY_FULL.code,
        id: release::ID,
        run_dir: release::RUN_DIR,
        dir: PathBuf::from(release::DATA_DIR).join("problems"),
        history: appliance.log_dir().join(logd::files::HISTORY),
        data: PathBuf::from(release::DATA_DIR),
        share: release::share_with_server,
    };
    // It returns only when it could not start.
    let e = problem::serve::run(host);
    eprintln!("the problem service did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsprobd is a service of a QS/VS host and runs on Linux only");
}

#[cfg(test)]
mod tests {
    use qvsmsg::problems::{CONDITIONS, RULES, kind as table};
    use refcode::Family;
    use supervise::control::about;

    /// Every attention and fatal code has its row, and every code a row
    /// names is one of the table's.
    #[test]
    fn the_table_holds_every_code_of_the_host() {
        for c in refcode::ALL {
            if matches!(c.family, Family::Attention | Family::Fatal) {
                assert!(RULES.condition(c.code).is_some(), "{} has no row", c.code);
            }
        }
        for row in CONDITIONS {
            let code = refcode::find(row.code).unwrap_or_else(|| panic!("{}", row.code));
            assert!(matches!(code.family, Family::Attention | Family::Fatal));
            if let Some(o) = &row.opens {
                for clear in o.cleared_by {
                    let c = refcode::find(clear).unwrap_or_else(|| panic!("{clear}"));
                    assert_eq!(c.family, Family::Progress, "{clear}");
                }
            }
        }
    }

    /// The table's kinds are the posting's.
    #[test]
    fn the_kinds_are_the_postings() {
        let pairs = [
            (table::DISK_UNIT, about::DISK_UNIT),
            (table::STORAGE_POOL, about::STORAGE_POOL),
            (table::VIRTUAL_DISK, about::VIRTUAL_DISK),
            (table::NETWORK_INTERFACE, about::NETWORK_INTERFACE),
            (table::VIRTUAL_SWITCH, about::VIRTUAL_SWITCH),
            (table::PARTITION, about::PARTITION),
            (table::CODE_SLOT, about::CODE_SLOT),
            (table::SERVICE, about::SERVICE),
        ];
        for (a, b) in pairs {
            assert_eq!(a, b);
        }
    }

    /// The severity is the condition's family's.
    #[test]
    fn a_fatal_code_is_terminal() {
        assert_eq!((RULES.severity)("B2003000"), problem::Severity::Terminal);
        assert_eq!((RULES.severity)("A2007020"), problem::Severity::Attention);
    }
}
