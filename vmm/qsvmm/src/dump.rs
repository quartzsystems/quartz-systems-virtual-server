// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A dump of the partition, taken from outside (`docs/DUMPS.md`): the
//! processors are stopped, their registers taken as they parked, and
//! the guest's memory written to the file the partition manager opened
//! and handed over. A partition that runs goes on afterwards; one that
//! is kept stays stopped, so its dump can be taken again.

use std::fs::File;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use dump::{Cpu, Header, Source};
use kvm_ioctls::VcpuFd;
use vm_memory::{Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryRegion};

use crate::machine::pause::{Pause, Reason};

/// How long the processors are given to stop for a dump.
const STOP_WAIT: Duration = Duration::from_secs(5);

/// A processor's registers as a dump has them; zero what cannot be read.
pub fn cpu_of(vcpu: &VcpuFd) -> Cpu {
    let mut cpu = Cpu::default();
    if let Ok(r) = vcpu.get_regs() {
        cpu.rax = r.rax;
        cpu.rbx = r.rbx;
        cpu.rcx = r.rcx;
        cpu.rdx = r.rdx;
        cpu.rsi = r.rsi;
        cpu.rdi = r.rdi;
        cpu.rbp = r.rbp;
        cpu.rsp = r.rsp;
        cpu.r8 = r.r8;
        cpu.r9 = r.r9;
        cpu.r10 = r.r10;
        cpu.r11 = r.r11;
        cpu.r12 = r.r12;
        cpu.r13 = r.r13;
        cpu.r14 = r.r14;
        cpu.r15 = r.r15;
        cpu.rip = r.rip;
        cpu.rflags = r.rflags;
    }
    if let Ok(s) = vcpu.get_sregs() {
        cpu.cs = u64::from(s.cs.selector);
        cpu.ss = u64::from(s.ss.selector);
        cpu.ds = u64::from(s.ds.selector);
        cpu.es = u64::from(s.es.selector);
        cpu.fs = u64::from(s.fs.selector);
        cpu.gs = u64::from(s.gs.selector);
        cpu.fs_base = s.fs.base;
        cpu.gs_base = s.gs.base;
        cpu.cr0 = s.cr0;
        cpu.cr2 = s.cr2;
        cpu.cr3 = s.cr3;
        cpu.cr4 = s.cr4;
        cpu.efer = s.efer;
        cpu.gdt_base = s.gdt.base;
        cpu.idt_base = s.idt.base;
    }
    cpu
}

/// The guest's memory, as the dump reads it.
struct Guest<'a>(&'a devices::Memory);

impl Source for Guest<'_> {
    fn regions(&self) -> Vec<(u64, u64)> {
        self.0.iter().map(|r| (r.start_addr().0, r.len())).collect()
    }

    fn read(&self, address: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0
            .read_slice(buf, GuestAddress(address))
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

/// What a dump needs of the running partition.
#[derive(Clone)]
pub struct Dumper {
    /// The guest's memory.
    pub memory: Arc<devices::Memory>,
    /// The processors' pause.
    pub pause: Arc<Pause>,
    /// The partition's name, ID and serial number.
    pub name: String,
    /// Its ID.
    pub id: u32,
    /// Its serial number.
    pub serial: String,
}

impl Dumper {
    /// Write a dump to `file` with the reference code `code` (and its
    /// text), taken at `time`, for `reason`. Its size and the bytes of
    /// memory written.
    pub fn take(
        &self,
        file: &File,
        code: &str,
        text: &str,
        time: u64,
        reason: &str,
    ) -> io::Result<dump::Written> {
        let kept = self.pause.kept();
        if !kept && !self.pause.hold_for(Reason::Dump, STOP_WAIT) {
            self.pause.release(Reason::Dump);
            return Err(io::Error::other("the processors did not stop"));
        }
        crate::post(&refcode::PARTITION_DUMP_STARTED, reason);
        let header = Header {
            partition: self.name.clone(),
            id: self.id,
            serial: self.serial.clone(),
            code: code.to_string(),
            text: text.to_string(),
            time,
            release: release::RELEASE.to_string(),
            reason: reason.to_string(),
        };
        let written = dump::write(
            file,
            &header,
            &self.pause.cpus(),
            &Guest(&self.memory),
            &mut |percent| {
                if percent < 100 {
                    crate::post(&refcode::PARTITION_DUMP_PROGRESS, &format!("{percent}%"));
                }
            },
        );
        if !kept {
            self.pause.release(Reason::Dump);
        }
        match &written {
            Ok(w) => crate::post(
                &refcode::PARTITION_DUMP_WRITTEN,
                &format!("{} bytes, {} of memory written", w.bytes, w.data),
            ),
            Err(e) => crate::post(&refcode::PARTITION_DUMP_FAILED, &e.to_string()),
        }
        written
    }
}
