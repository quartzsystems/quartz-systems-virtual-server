// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Locating the host's OVMF: the code image every boot uses, and the
//! variable store template a boot by firmware entry writes its entries
//! into. Both are host test tools standing in for a real machine's
//! firmware. The variables that name them are the harness's.

use std::env;
use std::path::{Path, PathBuf};

use crate::Error;

/// Package hint for the error message.
const HINT: &str = "Debian/Ubuntu: apt install ovmf";

/// Distribution paths tried when the environment variable is unset.
pub const DEFAULTS: &[&str] = &[
    "/usr/share/OVMF/OVMF_CODE_4M.fd",
    "/usr/share/OVMF/OVMF_CODE.fd",
    "/usr/share/edk2/ovmf/OVMF_CODE.fd",
    "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
];

/// The code image: the variable `code_env`, else the distribution
/// defaults.
pub fn locate(code_env: &str) -> Result<PathBuf, Error> {
    let mut tried = Vec::new();
    let found = match env::var_os(code_env) {
        Some(p) => {
            let p = PathBuf::from(p);
            tried.push(p.clone());
            p.is_file().then_some(p)
        }
        None => DEFAULTS.iter().map(PathBuf::from).find(|p| {
            tried.push(p.clone());
            p.is_file()
        }),
    };
    found.ok_or_else(|| Error::FirmwareNotFound {
        what: "code image",
        env: code_env.to_string(),
        tried,
        hint: HINT,
    })
}

/// The name of the variable store that goes with a code image: `VARS`
/// for `CODE`.
fn vars_beside(code: &Path) -> Option<PathBuf> {
    let name = code.file_name()?.to_str()?;
    let vars = code.with_file_name(name.replace("CODE", "VARS"));
    (vars != code).then_some(vars)
}

/// The variable store template that goes with `code`: the variable
/// `vars_env`, else the file beside the code image.
pub fn locate_vars(code: &Path, vars_env: &str) -> Result<PathBuf, Error> {
    let candidate = env::var_os(vars_env)
        .map(PathBuf::from)
        .or_else(|| vars_beside(code));
    match candidate {
        Some(p) if p.is_file() => Ok(p),
        other => Err(Error::FirmwareNotFound {
            what: "variable store",
            env: vars_env.to_string(),
            tried: other.into_iter().collect(),
            hint: HINT,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_store_is_named_after_the_code_image() {
        assert_eq!(
            vars_beside(Path::new("/usr/share/OVMF/OVMF_CODE_4M.fd")),
            Some(PathBuf::from("/usr/share/OVMF/OVMF_VARS_4M.fd"))
        );
        assert_eq!(vars_beside(Path::new("/x/OVMF.fd")), None);
    }

    #[test]
    fn a_variable_naming_no_file_is_an_error_that_names_it() {
        let e = locate_vars(Path::new("/x/OVMF.fd"), "APPLIANCE_TEST_NO_SUCH_VARS").unwrap_err();
        assert!(e.to_string().contains("APPLIANCE_TEST_NO_SUCH_VARS"), "{e}");
    }
}
