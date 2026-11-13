// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The model-specific registers a processor starts with. Everything
//! not named here is as KVM resets it.

use kvm_bindings::kvm_msr_entry;

const IA32_SYSENTER_CS: u32 = 0x174;
const IA32_SYSENTER_ESP: u32 = 0x175;
const IA32_SYSENTER_EIP: u32 = 0x176;
const IA32_TSC: u32 = 0x10;
const IA32_MISC_ENABLE: u32 = 0x1A0;
const MTRR_DEF_TYPE: u32 = 0x2FF;
const STAR: u32 = 0xC000_0081;
const LSTAR: u32 = 0xC000_0082;
const CSTAR: u32 = 0xC000_0083;
const SYSCALL_MASK: u32 = 0xC000_0084;
const KERNEL_GS_BASE: u32 = 0xC000_0102;

/// Fast string operations.
const MISC_ENABLE_FAST_STRING: u64 = 1;
/// MTRRs on, every address write-back: without it the kernel takes
/// all of memory for uncacheable.
const MTRR_ENABLE_WRITE_BACK: u64 = (1 << 11) | 6;

fn entry(index: u32, data: u64) -> kvm_msr_entry {
    kvm_msr_entry {
        index,
        data,
        ..Default::default()
    }
}

/// The registers set before a processor first runs.
pub fn boot() -> Vec<kvm_msr_entry> {
    vec![
        entry(IA32_SYSENTER_CS, 0),
        entry(IA32_SYSENTER_ESP, 0),
        entry(IA32_SYSENTER_EIP, 0),
        entry(STAR, 0),
        entry(CSTAR, 0),
        entry(KERNEL_GS_BASE, 0),
        entry(SYSCALL_MASK, 0),
        entry(LSTAR, 0),
        entry(IA32_TSC, 0),
        entry(IA32_MISC_ENABLE, MISC_ENABLE_FAST_STRING),
        entry(MTRR_DEF_TYPE, MTRR_ENABLE_WRITE_BACK),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_is_write_back_by_default() {
        let msrs = boot();
        let mtrr = msrs.iter().find(|m| m.index == MTRR_DEF_TYPE).unwrap();
        assert_eq!(mtrr.data, 0x806);
        let mut indexes: Vec<u32> = msrs.iter().map(|m| m.index).collect();
        indexes.sort_unstable();
        indexes.dedup();
        assert_eq!(indexes.len(), msrs.len());
    }
}
