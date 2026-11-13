// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A partition's CPUID: every feature KVM supports on this host, with
//! the topology leaves saying what the partition has: one package of
//! as many cores as it has processors, one thread a core. The
//! hypervisor leaves are KVM's own, so the guest uses its clock. One
//! thing the host has is withheld: hardware virtualization. A
//! partition does not run a hypervisor of its own.

use kvm_bindings::{KVM_CPUID_FLAG_SIGNIFCANT_INDEX, kvm_cpuid_entry2};

use crate::Error;
use crate::cpu::MAX_VCPUS;

/// KVM's signature leaf.
pub const KVM_SIGNATURE_LEAF: u32 = 0x4000_0000;
/// `KVMKVMKVM` in `ebx`, `ecx` and `edx`.
pub const KVM_SIGNATURE: [u32; 3] = [0x4B4D_564B, 0x564B_4D56, 0x0000_004D];

const LEAF_FEATURES: u32 = 1;
const LEAF_CACHE: u32 = 4;
const LEAF_TOPOLOGY: u32 = 0xB;
const LEAF_TOPOLOGY_V2: u32 = 0x1F;
const LEAF_AMD_FEATURES: u32 = 0x8000_0001;
const LEAF_AMD_SIZES: u32 = 0x8000_0008;
const LEAF_AMD_TOPOLOGY: u32 = 0x8000_001E;

const ECX_VMX: u32 = 1 << 5;
const ECX_X2APIC: u32 = 1 << 21;
const ECX_TSC_DEADLINE: u32 = 1 << 24;
const ECX_HYPERVISOR: u32 = 1 << 31;
const EDX_HTT: u32 = 1 << 28;
/// Leaf `8000_0001`: SVM.
const AMD_ECX_SVM: u32 = 1 << 2;

/// Bits of an APIC id that tell the cores of the package apart.
pub fn core_bits(count: u32) -> u32 {
    count.next_power_of_two().trailing_zeros()
}

fn topology(leaf: u32, level: u32, apic_id: u32, count: u32) -> kvm_cpuid_entry2 {
    // Level 0 is the thread (one a core), level 1 the core, beyond is none.
    let (eax, ebx, kind) = match level {
        0 => (0, 1, 1),
        1 => (core_bits(count), count, 2),
        _ => (0, 0, 0),
    };
    kvm_cpuid_entry2 {
        function: leaf,
        index: level,
        flags: KVM_CPUID_FLAG_SIGNIFCANT_INDEX,
        eax,
        ebx,
        ecx: (kind << 8) | level,
        edx: apic_id,
        ..Default::default()
    }
}

/// The CPUID of processor `index` of `count`, from what KVM supports.
pub fn for_vcpu(
    supported: &[kvm_cpuid_entry2],
    index: u32,
    count: u32,
) -> Result<Vec<kvm_cpuid_entry2>, Error> {
    if count == 0 || count > MAX_VCPUS || index >= count {
        return Err(Error::Processors(count));
    }
    let apic_id = index;
    let mut out: Vec<kvm_cpuid_entry2> = Vec::with_capacity(supported.len() + 8);
    let mut extended_topology = Vec::new();
    for entry in supported {
        let mut e = *entry;
        match e.function {
            LEAF_FEATURES => {
                e.ebx = (e.ebx & 0x0000_FFFF) | (count.min(255) << 16) | (apic_id << 24);
                e.ecx |= ECX_HYPERVISOR | ECX_TSC_DEADLINE | ECX_X2APIC;
                e.ecx &= !ECX_VMX;
                if count > 1 {
                    e.edx |= EDX_HTT;
                } else {
                    e.edx &= !EDX_HTT;
                }
            }
            LEAF_CACHE => {
                // No thread shares a cache; `count` cores in the package.
                if e.eax & 0x1F != 0 {
                    e.eax = (e.eax & 0x0000_3FFF) | ((count - 1) << 26);
                }
            }
            LEAF_TOPOLOGY | LEAF_TOPOLOGY_V2 => {
                if !extended_topology.contains(&e.function) {
                    extended_topology.push(e.function);
                }
                continue;
            }
            LEAF_AMD_FEATURES => e.ecx &= !AMD_ECX_SVM,
            LEAF_AMD_SIZES => {
                e.ecx = (e.ecx & !0xF0FF) | (core_bits(count) << 12) | (count - 1);
            }
            LEAF_AMD_TOPOLOGY => {
                e.eax = apic_id;
                e.ebx = (e.ebx & !0xFFFF) | apic_id;
                e.ecx &= !0x7FF;
            }
            _ => {}
        }
        out.push(e);
    }
    if !extended_topology.contains(&LEAF_TOPOLOGY) {
        extended_topology.push(LEAF_TOPOLOGY);
    }
    for leaf in extended_topology {
        for level in 0..3 {
            out.push(topology(leaf, level, apic_id, count));
        }
    }
    if !out.iter().any(|e| e.function == KVM_SIGNATURE_LEAF) {
        out.push(kvm_cpuid_entry2 {
            function: KVM_SIGNATURE_LEAF,
            eax: KVM_SIGNATURE_LEAF + 1,
            ebx: KVM_SIGNATURE[0],
            ecx: KVM_SIGNATURE[1],
            edx: KVM_SIGNATURE[2],
            ..Default::default()
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(function: u32, index: u32) -> kvm_cpuid_entry2 {
        kvm_cpuid_entry2 {
            function,
            index,
            ..Default::default()
        }
    }

    fn host() -> Vec<kvm_cpuid_entry2> {
        let mut features = leaf(1, 0);
        features.ebx = 0x00FF_0800 | 0x12;
        features.edx = EDX_HTT;
        features.ecx = ECX_VMX;
        let mut amd = leaf(0x8000_0001, 0);
        amd.ecx = AMD_ECX_SVM | 1;
        let mut cache = leaf(4, 0);
        cache.eax = 0xFC00_4121;
        vec![
            leaf(0, 0),
            features,
            cache,
            leaf(0xB, 0),
            leaf(0xB, 1),
            leaf(0x8000_0000, 0),
            amd,
        ]
    }

    #[test]
    fn each_processor_has_its_own_apic_id_and_the_package_s_count() {
        let cpuid = for_vcpu(&host(), 2, 4).unwrap();
        let features = cpuid.iter().find(|e| e.function == 1).unwrap();
        assert_eq!(features.ebx >> 24, 2);
        assert_eq!((features.ebx >> 16) & 0xFF, 4);
        assert_eq!(features.ebx & 0xFFFF, 0x0812);
        assert_ne!(features.ecx & ECX_HYPERVISOR, 0);
        assert_ne!(features.ecx & ECX_TSC_DEADLINE, 0);
        // Hardware virtualization is withheld, on either maker's processor.
        assert_eq!(features.ecx & ECX_VMX, 0);
        let amd = cpuid.iter().find(|e| e.function == 0x8000_0001).unwrap();
        assert_eq!(amd.ecx, 1);
        assert_ne!(features.edx & EDX_HTT, 0);
        let cache = cpuid.iter().find(|e| e.function == 4).unwrap();
        assert_eq!(cache.eax >> 26, 3);
        assert_eq!((cache.eax >> 14) & 0xFFF, 0);
    }

    #[test]
    fn the_topology_leaf_is_one_thread_a_core() {
        let cpuid = for_vcpu(&host(), 3, 4).unwrap();
        let levels: Vec<_> = cpuid.iter().filter(|e| e.function == 0xB).collect();
        assert_eq!(levels.len(), 3);
        assert_eq!((levels[0].eax, levels[0].ebx, levels[0].ecx), (0, 1, 0x100));
        assert_eq!((levels[1].eax, levels[1].ebx, levels[1].ecx), (2, 4, 0x201));
        assert_eq!(levels[2].ecx, 2);
        assert!(levels.iter().all(|l| l.edx == 3));
        assert_eq!(core_bits(1), 0);
        assert_eq!(core_bits(3), 2);
        assert_eq!(core_bits(4), 2);
    }

    #[test]
    fn one_processor_is_not_hyperthreaded_and_kvm_is_named() {
        let cpuid = for_vcpu(&host(), 0, 1).unwrap();
        let features = cpuid.iter().find(|e| e.function == 1).unwrap();
        assert_eq!(features.edx & EDX_HTT, 0);
        let kvm = cpuid
            .iter()
            .find(|e| e.function == KVM_SIGNATURE_LEAF)
            .unwrap();
        let mut name = Vec::new();
        for word in [kvm.ebx, kvm.ecx, kvm.edx] {
            name.extend(word.to_le_bytes());
        }
        assert_eq!(&name[..9], b"KVMKVMKVM");
    }

    #[test]
    fn a_count_out_of_range_is_refused() {
        assert_eq!(for_vcpu(&host(), 0, 0), Err(Error::Processors(0)));
        assert!(for_vcpu(&host(), 4, 4).is_err());
        assert!(for_vcpu(&host(), 0, 241).is_err());
    }
}
