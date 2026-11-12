// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The KVM probe: open `/dev/kvm`, check the API version and the
//! extensions a VMM cannot do without.

use kvm_bindings::KVM_API_VERSION;
use kvm_ioctls::{Cap, Kvm};

use super::{Report, Vendor};

/// The extensions `qsvmm` needs, with the words for a refusal.
const NEEDED: [(Cap, &str); 2] = [
    (Cap::UserMemory, "user memory regions"),
    (Cap::Irqchip, "an in-kernel interrupt controller"),
];

/// Probe the host.
pub fn probe() -> Report {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let vendor = Vendor::of_cpuinfo(&cpuinfo);
    match check() {
        Ok((api, max_vcpus)) => Report::Usable {
            vendor,
            api,
            max_vcpus,
        },
        Err(reason) => Report::NotAvailable { vendor, reason },
    }
}

/// The API version and the most vCPUs a partition may have.
fn check() -> Result<(i32, usize), String> {
    let kvm = Kvm::new().map_err(|e| format!("/dev/kvm: {e}"))?;
    let api = kvm.get_api_version();
    if u32::try_from(api) != Ok(KVM_API_VERSION) {
        return Err(format!("API version {api}, want {KVM_API_VERSION}"));
    }
    for (cap, what) in NEEDED {
        if !kvm.check_extension(cap) {
            return Err(format!("KVM lacks {what}"));
        }
    }
    // The capability itself: `get_max_vcpus` would fall back to a guess.
    let max = kvm.check_extension_int(Cap::MaxVcpus);
    match usize::try_from(max) {
        Ok(n) if n > 0 => Ok((api, n)),
        _ => Err("KVM does not report its maximum vCPU count".into()),
    }
}
