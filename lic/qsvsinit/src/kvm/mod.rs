// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Whether this host can run partitions: the report PID 1 prints. The
//! probe itself (`probe`) needs `/dev/kvm`.

#[cfg(target_os = "linux")]
pub mod probe;

use std::fmt;

/// The processor's virtualization extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    /// Intel VT-x.
    Vmx,
    /// AMD-V.
    Svm,
    /// The processor reports neither.
    None,
}

impl Vendor {
    /// From `/proc/cpuinfo`: the `vmx` or `svm` flag of the first processor.
    pub fn of_cpuinfo(cpuinfo: &str) -> Self {
        let flags = cpuinfo
            .lines()
            .find(|l| l.starts_with("flags"))
            .and_then(|l| l.split_once(':'))
            .map_or("", |(_, flags)| flags);
        let has = |flag: &str| flags.split_ascii_whitespace().any(|f| f == flag);
        if has("vmx") {
            Vendor::Vmx
        } else if has("svm") {
            Vendor::Svm
        } else {
            Vendor::None
        }
    }

    fn name(self) -> &'static str {
        match self {
            Vendor::Vmx => "VMX",
            Vendor::Svm => "SVM",
            Vendor::None => "neither VMX nor SVM",
        }
    }
}

/// What the probe found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// `/dev/kvm` opens and has everything a VMM needs.
    Usable {
        /// VMX or SVM.
        vendor: Vendor,
        /// The KVM API version.
        api: i32,
        /// The most vCPUs one partition may have.
        max_vcpus: usize,
    },
    /// It does not, and why.
    NotAvailable {
        /// What the processor reports.
        vendor: Vendor,
        /// The reason.
        reason: String,
    },
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Report::Usable {
                vendor,
                api,
                max_vcpus,
            } => write!(
                f,
                "KVM usable: {}, API version {api}, {max_vcpus} vCPUs at most",
                vendor.name()
            ),
            Report::NotAvailable { vendor, reason } => write!(
                f,
                "KVM not available: {reason} (processor reports {})",
                vendor.name()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendor_is_a_whole_flag_of_the_flags_line() {
        let intel = "model name\t: vmxish\nflags\t\t: fpu vme vmx smx\n";
        let amd = "flags\t\t: fpu svm svm_lock\nflags\t\t: fpu\n";
        assert_eq!(Vendor::of_cpuinfo(intel), Vendor::Vmx);
        assert_eq!(Vendor::of_cpuinfo(amd), Vendor::Svm);
        assert_eq!(Vendor::of_cpuinfo("flags\t: fpu svm_lock\n"), Vendor::None);
        assert_eq!(Vendor::of_cpuinfo(""), Vendor::None);
    }

    #[test]
    fn the_lines_are_what_the_harness_looks_for() {
        let ok = Report::Usable {
            vendor: Vendor::Vmx,
            api: 12,
            max_vcpus: 1024,
        };
        assert_eq!(
            ok.to_string(),
            "KVM usable: VMX, API version 12, 1024 vCPUs at most"
        );
        let no = Report::NotAvailable {
            vendor: Vendor::None,
            reason: "/dev/kvm: No such file or directory".into(),
        };
        assert_eq!(
            no.to_string(),
            "KVM not available: /dev/kvm: No such file or directory \
             (processor reports neither VMX nor SVM)"
        );
    }
}
