// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 1: boot to the status screen. Base ready within 10 s of the
//! kernel starting, under KVM.

use release::{fact, screen};

use super::{Run, all_ready, base_ready, channel, fact_of, posted, screen_with};

/// Seconds from the kernel starting to "Base ready", under KVM.
const READY_WITHIN: f64 = 10.0;

/// The suite.
pub fn status(run: &Run<'_>) -> Result<String, String> {
    let launch = run.launch();
    let mut guest = run.boot(&launch)?;
    let deadline = run.deadline();
    // Every step of the IPL posts its code, in order.
    for code in [
        &refcode::IPL_STARTED,
        &refcode::MOUNTED,
        &refcode::CGROUPS,
        &refcode::RECORD_READ,
        &refcode::DATA_MOUNTED,
        &refcode::CLOCK_SET,
        &refcode::SEED_CHECKED,
        &refcode::STATE_LOADED,
        &refcode::SERVICES_STARTED,
    ] {
        guest.expect(&posted(code, ""), deadline)?;
    }
    let up = base_ready(&mut guest, deadline)?;
    if launch.kvm && up >= READY_WITHIN {
        return Err(format!(
            "base ready {up} s after the kernel started; {READY_WITHIN} s is allowed"
        ));
    }
    // The screen is drawn again with the code and every service ready.
    let rows = [
        screen::row(screen::REFERENCE, refcode::BASE_READY.code),
        screen::service("logd", "ready"),
        screen::service("netd", "ready"),
    ];
    screen_with(&mut guest, &rows, deadline)?;

    let status = guest.channel(deadline)?.status()?;
    all_ready(&status)?;
    for (key, wanted) in [
        (fact::PRODUCT, release::NAME.to_string()),
        (fact::RELEASE, release::RELEASE.to_string()),
        (fact::IMAGE, "test".to_string()),
        (fact::SLOT, "A".to_string()),
        (fact::RECORD, "default A, nothing pending".to_string()),
    ] {
        let found = fact_of(&status, key)?;
        if found != wanted {
            return Err(format!("the status's {key} is `{found}`, not `{wanted}`"));
        }
    }
    let reference = fact_of(&status, fact::REFERENCE)?;
    if !reference.starts_with(refcode::BASE_READY.code) {
        return Err(format!("the current reference code is `{reference}`"));
    }
    if !fact_of(&status, fact::DATA)?.starts_with("/dev/") {
        return Err(format!("/data is `{}`", fact_of(&status, fact::DATA)?));
    }
    // The first IPL of this release notes it in the record.
    let record = channel(&mut guest)?.status()?;
    let host = fact_of(&status, fact::HOST)?.to_string();
    let kvm = if launch.kvm {
        "under KVM"
    } else {
        "without KVM"
    };
    Ok(format!(
        "base ready {up} s after the kernel started ({kvm}); host {host}; {} services ready; {}",
        record.services.len(),
        fact_of(&status, fact::MANAGEMENT)?
    ))
}
