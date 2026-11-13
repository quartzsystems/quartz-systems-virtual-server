// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The boot processor's registers at the kernel's 64-bit entry.

use kvm_bindings::{kvm_fpu, kvm_regs, kvm_segment, kvm_sregs};

use crate::boot::tables::{CODE_SELECTOR, DATA_SELECTOR, GDT_LIMIT, TSS_SELECTOR};
use crate::layout::{BOOT_STACK, GDT_START, PML4_START, ZERO_PAGE_START};

const CR0_PE: u64 = 1;
const CR0_ET: u64 = 1 << 4;
const CR0_PG: u64 = 1 << 31;
const CR4_PAE: u64 = 1 << 5;
const EFER_LME: u64 = 1 << 8;
const EFER_LMA: u64 = 1 << 10;

/// The general registers: `rip` at the entry, `rsi` at the zero page,
/// interrupts off.
pub fn general(entry: u64) -> kvm_regs {
    kvm_regs {
        rflags: 2,
        rip: entry,
        rsp: BOOT_STACK,
        rbp: BOOT_STACK,
        rsi: ZERO_PAGE_START,
        ..Default::default()
    }
}

fn segment(selector: u16, type_: u8, long: bool) -> kvm_segment {
    kvm_segment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector,
        type_,
        present: 1,
        dpl: 0,
        db: u8::from(!long),
        s: 1,
        l: u8::from(long),
        g: 1,
        ..Default::default()
    }
}

/// The special registers, starting from what KVM gave the processor:
/// long mode, paging on the tables of [`crate::boot::tables`], flat
/// segments.
pub fn special(mut sregs: kvm_sregs) -> kvm_sregs {
    let data = segment(DATA_SELECTOR, 3, false);
    sregs.cs = segment(CODE_SELECTOR, 11, true);
    sregs.ds = data;
    sregs.es = data;
    sregs.fs = data;
    sregs.gs = data;
    sregs.ss = data;
    sregs.tr = kvm_segment {
        base: 0,
        limit: 0xF_FFFF,
        selector: TSS_SELECTOR,
        type_: 11,
        present: 1,
        g: 1,
        ..Default::default()
    };
    sregs.gdt.base = GDT_START;
    sregs.gdt.limit = GDT_LIMIT;
    sregs.idt.base = 0;
    sregs.idt.limit = 0;
    sregs.cr0 |= CR0_PE | CR0_ET | CR0_PG;
    sregs.cr3 = PML4_START;
    sregs.cr4 |= CR4_PAE;
    sregs.efer |= EFER_LME | EFER_LMA;
    sregs
}

/// The floating point state: the defaults of a reset.
pub fn fpu() -> kvm_fpu {
    kvm_fpu {
        fcw: 0x37F,
        mxcsr: 0x1F80,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_processor_starts_in_long_mode_at_the_entry() {
        let regs = general(0x100_0200);
        assert_eq!(regs.rip, 0x100_0200);
        assert_eq!(regs.rsi, ZERO_PAGE_START);
        // Interrupts off: bit 9 clear; bit 1 is always set.
        assert_eq!(regs.rflags, 2);

        let sregs = special(kvm_sregs::default());
        assert_eq!((sregs.cs.l, sregs.cs.db, sregs.cs.selector), (1, 0, 0x10));
        assert_eq!((sregs.ds.l, sregs.ds.db, sregs.ss.selector), (0, 1, 0x18));
        assert_eq!(sregs.cr3, PML4_START);
        assert_eq!(sregs.cr0 & (CR0_PE | CR0_PG), CR0_PE | CR0_PG);
        assert_ne!(sregs.cr4 & CR4_PAE, 0);
        assert_eq!(sregs.efer, EFER_LME | EFER_LMA);
    }
}
