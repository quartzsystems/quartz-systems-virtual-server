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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use arch::boot::regs;
use arch::cpu::{cpuid, lapic, msr};
use devices::Memory;
use devices::bus::Bus;
use devices::pci::PciBus;
use kvm_bindings::{
    CpuId, KVM_MAX_CPUID_ENTRIES, KVM_SYSTEM_EVENT_CRASH, KVM_SYSTEM_EVENT_RESET,
    KVM_SYSTEM_EVENT_SHUTDOWN, Msrs, kvm_msr_entry,
};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use vm_memory::{Bytes, GuestAddress};

use crate::end::End;

fn kvm_error(what: &'static str) -> impl Fn(kvm_ioctls::Error) -> io::Error {
    move |e| {
        io::Error::other(format!(
            "{what}: {}",
            io::Error::from_raw_os_error(e.errno())
        ))
    }
}

/// Make virtual CPU `index` of `count`, `threads` of them to a core,
/// and set what it starts with; the first enters the kernel at `entry`.
pub fn create(
    kvm: &Kvm,
    vm: &VmFd,
    index: u32,
    count: u32,
    threads: u32,
    entry: u64,
) -> io::Result<VcpuFd> {
    let vcpu = vm
        .create_vcpu(u64::from(index))
        .map_err(kvm_error("a processor"))?;
    let supported = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(kvm_error("the supported CPUID"))?;
    let entries =
        cpuid::for_vcpu(supported.as_slice(), index, count, threads).map_err(io::Error::other)?;
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
    /// Set to have the processor say what state it is in.
    pub report: Arc<AtomicBool>,
    /// The guest's memory, for the code a report shows.
    pub memory: Arc<Memory>,
    /// The host thread a dedicated processor's thread stays on.
    pub pin: Option<u32>,
}

/// The signal that brings a processor's thread out of the guest. Its
/// handler does nothing: coming out is the point.
pub fn kick_signal() -> i32 {
    vmm_sys_util::signal::SIGRTMIN()
}

extern "C" fn kicked(_: libc::c_int, _: *mut libc::siginfo_t, _: *mut libc::c_void) {}

/// Have [`kick_signal`] interrupt a thread and do nothing else.
pub fn install_kick() -> io::Result<()> {
    vmm_sys_util::signal::register_signal_handler(kick_signal(), kicked)
        .map_err(|e| io::Error::from_raw_os_error(e.errno()))
}

/// The local APIC's timer registers.
const LVT_TIMER: usize = 0x320;
const TIMER_INITIAL: usize = 0x380;
const TIMER_CURRENT: usize = 0x390;
/// The TSC, and the deadline the timer is armed for.
const IA32_TSC: u32 = 0x10;
const IA32_TSC_DEADLINE: u32 = 0x6E0;

/// Read guest memory at a virtual address of the processor's.
fn read_virtual(vcpu: &VcpuFd, memory: &Memory, address: u64, bytes: &mut [u8]) -> bool {
    vcpu.translate_gva(address)
        .ok()
        .filter(|t| t.valid != 0)
        .is_some_and(|t| {
            memory
                .read_slice(bytes, GuestAddress(t.physical_address))
                .is_ok()
        })
}

/// The text a processor that takes no interrupts has on its stack:
/// what the words there point at, near the code, when it is printable.
/// A kernel that stopped itself early names its reason this way and no
/// other. Everything read is the guest's: bounded, and only shown.
fn said(vcpu: &VcpuFd, memory: &Memory, registers: &[u64], rsp: u64, rip: u64) -> Vec<String> {
    const WORDS: u64 = 512;
    const NEAR: u64 = 16 << 20;
    const MOST: usize = 6;
    let mut pointers = registers.to_vec();
    for word in 0..WORDS {
        let mut raw = [0u8; 8];
        if !read_virtual(vcpu, memory, rsp.wrapping_add(word * 8), &mut raw) {
            break;
        }
        pointers.push(u64::from_le_bytes(raw));
    }
    let mut found: Vec<String> = Vec::new();
    for pointer in pointers {
        if pointer.abs_diff(rip) > NEAR {
            continue;
        }
        let mut text = [0u8; 96];
        if !read_virtual(vcpu, memory, pointer, &mut text) {
            continue;
        }
        let printable: String = text
            .iter()
            .take_while(|b| (0x20..0x7f).contains(*b))
            .map(|b| char::from(*b))
            .collect();
        let words = printable.split(' ').filter(|w| w.len() > 2).count();
        if printable.len() >= 12 && words >= 2 && !found.contains(&printable) {
            found.push(printable);
            if found.len() == MOST {
                break;
            }
        }
    }
    found
}

/// What state a processor is in, in a line: where it is, whether it
/// takes interrupts, whether it is halted, what its timer is armed
/// for, and the bytes it is about to run. For a partition that has
/// gone quiet.
fn state(vcpu: &VcpuFd, index: u32, memory: &Memory) -> String {
    let mut parts = vec![format!("qsvmm: vcpu{index}")];
    if let Ok(r) = vcpu.get_regs() {
        let interrupts = if r.rflags & 0x200 == 0 { "off" } else { "on" };
        parts.push(format!(
            "rip {:#x} rsp {:#x} rflags {:#x} interrupts {interrupts}",
            r.rip, r.rsp, r.rflags
        ));
        let mut bytes = [0u8; 8];
        if read_virtual(vcpu, memory, r.rip, &mut bytes) {
            parts.push(format!("code {bytes:02x?}"));
        } else {
            parts.push("code not mapped".into());
        }
        if r.rflags & 0x200 == 0 {
            let registers = [
                r.rax, r.rbx, r.rcx, r.rdx, r.rsi, r.rdi, r.rbp, r.r8, r.r9, r.r10, r.r11, r.r12,
                r.r13, r.r14, r.r15,
            ];
            parts.push(format!("registers {registers:x?}"));
            let mut stack = Vec::new();
            for word in 0..48u64 {
                let mut raw = [0u8; 8];
                if !read_virtual(vcpu, memory, r.rsp.wrapping_add(word * 8), &mut raw) {
                    break;
                }
                stack.push(u64::from_le_bytes(raw));
            }
            parts.push(format!("stack {stack:x?}"));
            let said = said(vcpu, memory, &registers, r.rsp, r.rip);
            if !said.is_empty() {
                parts.push(format!("names {said:?}"));
            }
        }
    }
    if let Ok(s) = vcpu.get_sregs() {
        parts.push(format!(
            "cr0 {:#x} cr2 {:#x} cr3 {:#x} cr4 {:#x} efer {:#x}",
            s.cr0, s.cr2, s.cr3, s.cr4, s.efer
        ));
    }
    if let Ok(mp) = vcpu.get_mp_state() {
        let name = match mp.mp_state {
            0 => "runnable",
            1 => "not started",
            2 => "init received",
            3 => "halted",
            4 => "sipi received",
            _ => "other",
        };
        parts.push(format!("state {name}"));
    }
    if let Ok(apic) = vcpu.get_lapic() {
        parts.push(format!(
            "timer lvt {:#x} initial {:#x} current {:#x}",
            lapic::register(&apic, LVT_TIMER),
            lapic::register(&apic, TIMER_INITIAL),
            lapic::register(&apic, TIMER_CURRENT)
        ));
    }
    let wanted = [IA32_TSC, IA32_TSC_DEADLINE].map(|index| kvm_msr_entry {
        index,
        ..Default::default()
    });
    if let Ok(mut msrs) = Msrs::from_entries(&wanted)
        && vcpu.get_msrs(&mut msrs).is_ok()
    {
        let values = msrs.as_slice();
        parts.push(format!(
            "tsc {:#x} deadline {:#x}",
            values[0].data, values[1].data
        ));
    }
    parts.join(" ")
}

fn run(mut vcpu: VcpuFd, index: u32, wiring: &Wiring) {
    let end = |end: End, why: String| {
        let _ = wiring.ends.send((end, format!("processor {index}: {why}")));
    };
    if let Some(cpu) = wiring.pin
        && let Err(e) = crate::sys::pin_this_thread(cpu)
    {
        return end(
            End::VmmError,
            format!("not pinned to host thread {cpu}: {e}"),
        );
    }
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
            Err(e) if matches!(e.errno(), libc::EINTR | libc::EAGAIN) => {
                if wiring.report.swap(false, Ordering::Relaxed) {
                    eprintln!("{}", state(&vcpu, index, &wiring.memory));
                }
            }
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
