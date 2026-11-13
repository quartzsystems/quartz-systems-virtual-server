// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The rows QS/VS adds to `buildkit`'s doctor: KVM and nested
//! virtualization on the build host (the test boots a hypervisor), the
//! OVMF firmware, the QS/400 image the partition suites IPL, and the
//! license headers.

use std::fs;

use buildkit::Ctx;
use buildkit::host::doctor::{Row, Status};
use conformance::suite::ALLOW_NO_KVM_ENV;
use conformance::suite::partition::{QS400_IMAGE_ENV, qs400_image};
use conformance::{firmware, qemu};

/// The `nested` parameter of whichever KVM vendor module is loaded.
const NESTED: [&str; 2] = [
    "/sys/module/kvm_intel/parameters/nested",
    "/sys/module/kvm_amd/parameters/nested",
];

fn row(name: &str, ok: bool, detail: String, required: bool) -> Row {
    let status = match (ok, required) {
        (true, _) => Status::Ok,
        (false, true) => Status::Missing,
        (false, false) => Status::Warn,
    };
    Row {
        name: name.to_string(),
        required,
        status,
        detail,
    }
}

/// Whether a `nested` parameter's text says it is on.
fn nested_on(text: &str) -> bool {
    matches!(text.trim(), "Y" | "y" | "1")
}

fn kvm(required: bool) -> Row {
    let ok = qemu::kvm_usable();
    let detail = if ok {
        "/dev/kvm opens read-write".to_string()
    } else {
        format!(
            "/dev/kvm is not usable (join the kvm group); the guest then has no KVM \
             and `test` fails unless {ALLOW_NO_KVM_ENV}=1"
        )
    };
    row("KVM on the build host", ok, detail, required)
}

fn nested(required: bool) -> Row {
    let found = NESTED
        .iter()
        .find_map(|p| fs::read_to_string(p).ok().map(|t| (*p, t)));
    let (ok, detail) = match found {
        Some((path, text)) if nested_on(&text) => (true, format!("{path} = {}", text.trim())),
        Some((path, text)) => (
            false,
            format!(
                "{path} = {}; the guest will not see VMX or SVM",
                text.trim()
            ),
        ),
        None => (
            false,
            "neither kvm_intel nor kvm_amd has a nested parameter".to_string(),
        ),
    };
    row("nested virtualization", ok, detail, required)
}

fn ovmf() -> Row {
    let name = format!("OVMF ({})", firmware::CODE_ENV);
    match firmware::locate() {
        Ok(code) => {
            // The slot B boot writes its boot entry into the variable store.
            match firmware::locate_vars(&code) {
                Ok(vars) => row(
                    &name,
                    true,
                    format!("{}, {}", code.display(), vars.display()),
                    true,
                ),
                Err(e) => row(&name, false, e.to_string(), true),
            }
        }
        Err(e) => row(&name, false, e.to_string(), true),
    }
}

/// The QS/400 disk image the partition suites IPL. Without it those
/// suites cannot run; the rest of the pipeline does not need it.
fn guest_image() -> Row {
    let name = format!("QS/400 guest image ({QS400_IMAGE_ENV})");
    match qs400_image() {
        Ok(path) => row(
            &name,
            true,
            format!("{} has an ESP and a LIC partition", path.display()),
            false,
        ),
        Err(e) => row(&name, false, e, false),
    }
}

fn headers(ctx: &Ctx) -> Row {
    let name = "license headers";
    match super::header::check(&ctx.paths.repo) {
        Ok((count, bad)) if bad.is_empty() => row(
            name,
            true,
            format!("{count} .rs files start with the header"),
            true,
        ),
        Ok((_, bad)) => row(name, false, format!("no header: {}", bad.join(", ")), true),
        Err(e) => row(name, false, format!("{e:#}"), true),
    }
}

fn neutral(ctx: &Ctx) -> Row {
    let name = "base/ is product-neutral";
    match super::neutral::check(&ctx.paths.repo, ctx.product.id) {
        Ok((count, problems)) if problems.is_empty() => row(
            name,
            true,
            format!("{count} crates name no product and depend on nothing of lic/"),
            true,
        ),
        Ok((_, problems)) => row(name, false, problems.join("; "), true),
        Err(e) => row(name, false, format!("{e:#}"), true),
    }
}

/// QS/VS's rows. KVM and nested virtualization are required unless
/// `QSVS_ALLOW_NO_KVM=1` says the host has neither.
pub fn rows(ctx: &Ctx) -> Vec<Row> {
    let required = !std::env::var(ALLOW_NO_KVM_ENV).is_ok_and(|v| v == "1");
    vec![
        kvm(required),
        nested(required),
        ovmf(),
        guest_image(),
        headers(ctx),
        neutral(ctx),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_is_y_or_1() {
        assert!(nested_on("Y\n"));
        assert!(nested_on("1\n"));
        assert!(!nested_on("N\n"));
        assert!(!nested_on("0\n"));
    }

    #[test]
    fn a_failed_check_is_missing_only_when_required() {
        assert_eq!(row("x", false, String::new(), true).status, Status::Missing);
        assert_eq!(row("x", false, String::new(), false).status, Status::Warn);
        assert_eq!(row("x", true, String::new(), true).status, Status::Ok);
    }
}
