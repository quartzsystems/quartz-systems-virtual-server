// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Locating the host's OVMF: the code image every boot uses, and the
//! variable store template the slot B boot writes its boot entry into.
//! Both are host test tools standing in for a real machine's firmware.

use std::env;
use std::path::{Path, PathBuf};

use crate::error::HarnessError;

/// Environment variable naming the OVMF code image.
pub const CODE_ENV: &str = "QSVS_OVMF_CODE";
/// Environment variable naming OVMF's variable store template.
pub const VARS_ENV: &str = "QSVS_OVMF_VARS";

/// Package hint for the error message.
const HINT: &str = "Debian/Ubuntu: apt install ovmf";

/// Distribution paths tried when the environment variable is unset.
const DEFAULTS: &[&str] = &[
    "/usr/share/OVMF/OVMF_CODE_4M.fd",
    "/usr/share/OVMF/OVMF_CODE.fd",
    "/usr/share/edk2/ovmf/OVMF_CODE.fd",
    "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
];

/// The code image: the environment variable, else the distribution defaults.
pub fn locate() -> Result<PathBuf, HarnessError> {
    let mut tried = Vec::new();
    let found = match env::var_os(CODE_ENV) {
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
    found.ok_or(HarnessError::FirmwareNotFound {
        what: "code image",
        env: CODE_ENV,
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

/// The variable store template that goes with `code`: the environment
/// variable, else the file beside the code image.
pub fn locate_vars(code: &Path) -> Result<PathBuf, HarnessError> {
    let candidate = env::var_os(VARS_ENV)
        .map(PathBuf::from)
        .or_else(|| vars_beside(code));
    match candidate {
        Some(p) if p.is_file() => Ok(p),
        other => Err(HarnessError::FirmwareNotFound {
            what: "variable store",
            env: VARS_ENV,
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
}
