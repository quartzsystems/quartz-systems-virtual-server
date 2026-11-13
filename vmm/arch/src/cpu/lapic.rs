// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The local APIC's local interrupt pins: LINT0 takes external
//! interrupts and LINT1 the NMI, as a board wires them.

use kvm_bindings::kvm_lapic_state;

const LVT_LINT0: usize = 0x350;
const LVT_LINT1: usize = 0x360;
const MODE_MASK: u32 = 0x700;
const MODE_EXTINT: u32 = 0x700;
const MODE_NMI: u32 = 0x400;

/// The register at byte `at` of the APIC page.
pub fn register(state: &kvm_lapic_state, at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    for (b, r) in bytes.iter_mut().zip(&state.regs[at..at + 4]) {
        *b = r.to_ne_bytes()[0];
    }
    u32::from_le_bytes(bytes)
}

fn set_register(state: &mut kvm_lapic_state, at: usize, value: u32) {
    for (r, b) in state.regs[at..at + 4].iter_mut().zip(value.to_le_bytes()) {
        *r = i8::from_ne_bytes([b]);
    }
}

/// The APIC state `state` with the two pins set.
pub fn with_pins(mut state: kvm_lapic_state) -> kvm_lapic_state {
    let lint0 = register(&state, LVT_LINT0);
    set_register(&mut state, LVT_LINT0, (lint0 & !MODE_MASK) | MODE_EXTINT);
    let lint1 = register(&state, LVT_LINT1);
    set_register(&mut state, LVT_LINT1, (lint1 & !MODE_MASK) | MODE_NMI);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pins_take_their_modes_and_keep_their_other_bits() {
        let mut state = kvm_lapic_state::default();
        set_register(&mut state, LVT_LINT0, 0x0001_0000);
        let state = with_pins(state);
        assert_eq!(register(&state, LVT_LINT0), 0x0001_0700);
        assert_eq!(register(&state, LVT_LINT1), 0x0000_0400);
    }
}
