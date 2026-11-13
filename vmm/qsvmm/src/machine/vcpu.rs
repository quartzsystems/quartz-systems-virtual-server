// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A processor: its state before it first runs, and the loop that
//! runs it. An exit for a port or a memory address goes to the device
//! there; an exit that ends the partition is told to the main thread,
//! which ends the process.
//!
//! Only processor 0 is given registers: the others wait, as KVM
//! leaves them, for the kernel on processor 0 to start them.

use std::io;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use arch::boot::regs;
use arch::cpu::{cpuid, lapic, msr};
use devices::bus::Bus;
use devices::pci::PciBus;
use kvm_bindings::{
    CpuId, KVM_MAX_CPUID_ENTRIES, KVM_SYSTEM_EVENT_CRASH, KVM_SYSTEM_EVENT_RESET,
    KVM_SYSTEM_EVENT_SHUTDOWN, Msrs,
};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};

use crate::end::End;

fn kvm_error(what: &'static str) -> impl Fn(kvm_ioctls::Error) -> io::Error {
    move |e| {
        io::Error::other(format!(
            "{what}: {}",
            io::Error::from_raw_os_error(e.errno())
        ))
    }
}

/// Make processor `index` of `count` and set what it starts with;
/// processor 0 enters the kernel at `entry`.
pub fn create(kvm: &Kvm, vm: &VmFd, index: u32, count: u32, entry: u64) -> io::Result<VcpuFd> {
    let vcpu = vm
        .create_vcpu(u64::from(index))
        .map_err(kvm_error("a processor"))?;
    let supported = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(kvm_error("the supported CPUID"))?;
    let entries = cpuid::for_vcpu(supported.as_slice(), index, count).map_err(io::Error::other)?;
    let cpuid = CpuId::from_entries(&entries).map_err(|_| io::Error::other("the CPUID table"))?;
    vcpu.set_cpuid2(&cpuid).map_err(kvm_error("the CPUID"))?;

    let msrs = Msrs::from_entries(&msr::boot()).map_err(|_| io::Error::other("the MSR table"))?;
    let set = vcpu.set_msrs(&msrs).map_err(kvm_error("the MSRs"))?;
    if set != msr::boot().len() {
        return Err(io::Error::other("an MSR was refused"));
    }
    let apic = vcpu.get_lapic().map_err(kvm_error("the local APIC"))?;
    vcpu.set_lapic(&lapic::with_pins(apic))
        .map_err(kvm_error("the local APIC"))?;
    vcpu.set_fpu(&regs::fpu()).map_err(kvm_error("the FPU"))?;
    if index == 0 {
        let sregs = vcpu
            .get_sregs()
            .map_err(kvm_error("the special registers"))?;
        vcpu.set_sregs(&regs::special(sregs))
            .map_err(kvm_error("the special registers"))?;
        vcpu.set_regs(&regs::general(entry))
            .map_err(kvm_error("the registers"))?;
    }
    Ok(vcpu)
}

/// What a processor's thread works with.
pub struct Wiring {
    /// The I/O ports.
    pub io: Arc<Bus>,
    /// The PCI bus.
    pub pci: Arc<Mutex<PciBus>>,
    /// Where an end is told.
    pub ends: Sender<(End, String)>,
}

fn run(mut vcpu: VcpuFd, index: u32, wiring: &Wiring) {
    let end = |end: End, why: String| {
        let _ = wiring.ends.send((end, format!("processor {index}: {why}")));
    };
    loop {
        match vcpu.run() {
            Ok(VcpuExit::IoIn(port, data)) => {
                wiring.io.read(u64::from(port), data);
            }
            Ok(VcpuExit::IoOut(port, data)) => {
                wiring.io.write(u64::from(port), data);
            }
            Ok(VcpuExit::MmioRead(address, data)) => match wiring.pci.lock() {
                Ok(pci) => {
                    pci.read_memory(address, data);
                }
                Err(_) => data.fill(0xFF),
            },
            Ok(VcpuExit::MmioWrite(address, data)) => {
                if let Ok(pci) = wiring.pci.lock() {
                    pci.write_memory(address, data);
                }
            }
            // With the interrupt controller in the kernel a halt
            // stays there; one that comes out has nothing to wake it.
            Ok(VcpuExit::Hlt) => {
                return end(End::GuestFault, "halted with nothing to wake it".into());
            }
            Ok(VcpuExit::Shutdown) => return end(End::GuestFault, "triple fault".into()),
            Ok(VcpuExit::SystemEvent(KVM_SYSTEM_EVENT_SHUTDOWN, _)) => {
                return end(End::PoweredOff, "system event".into());
            }
            Ok(VcpuExit::SystemEvent(KVM_SYSTEM_EVENT_RESET, _)) => {
                return end(End::Restart, "system event".into());
            }
            Ok(VcpuExit::SystemEvent(KVM_SYSTEM_EVENT_CRASH, _)) => {
                return end(End::GuestFault, "the guest reported a crash".into());
            }
            Ok(VcpuExit::FailEntry(reason, _)) => {
                return end(End::GuestFault, format!("entry failed, reason {reason:#x}"));
            }
            Ok(VcpuExit::InternalError) => return end(End::VmmError, "KVM internal error".into()),
            Ok(other) => return end(End::VmmError, format!("unexpected exit {other:?}")),
            Err(e) if matches!(e.errno(), libc::EINTR | libc::EAGAIN) => {}
            Err(e) => {
                return end(
                    End::VmmError,
                    format!("KVM_RUN: {}", io::Error::from_raw_os_error(e.errno())),
                );
            }
        }
    }
}

/// Start a processor's thread.
pub fn spawn(vcpu: VcpuFd, index: u32, wiring: Wiring) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name(format!("vcpu{index}"))
        .spawn(move || run(vcpu, index, &wiring))
}
