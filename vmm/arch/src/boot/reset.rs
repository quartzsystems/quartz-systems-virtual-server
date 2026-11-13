// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The reset vector. A partition has no firmware, and a Linux kernel
//! on hardware-reduced ACPI restarts by EFI's reset service or, with
//! none, by a jump to the reset vector; it does not try the ACPI reset
//! register. Seven bytes there pulse the keyboard controller's reset
//! line, which ends the partition for a restart. They run only when a
//! guest jumps there, never before the kernel.

use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

use crate::Error;
use crate::layout::{I8042_COMMAND_PORT, I8042_RESET, RESET_VECTOR};

/// `mov al, FE`; `out 64, al`; `hlt`; `jmp` back to the `hlt`.
#[allow(clippy::cast_possible_truncation)] // the port is below 256
pub const STUB: [u8; 7] = [
    0xB0,
    I8042_RESET,
    0xE6,
    I8042_COMMAND_PORT as u8,
    0xF4,
    0xEB,
    0xFD,
];

/// Write the stub at the reset vector.
pub fn write(mem: &GuestMemoryMmap) -> Result<(), Error> {
    mem.write_slice(&STUB, GuestAddress(RESET_VECTOR))
        .map_err(|_| Error::GuestMemory("the reset vector"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{HIGH_RAM_START, Kind, Ram};

    #[test]
    fn the_stub_pulses_the_reset_line_and_fits_before_the_megabyte() {
        assert_eq!(STUB, [0xB0, 0xFE, 0xE6, 0x64, 0xF4, 0xEB, 0xFD]);
        assert!(RESET_VECTOR + STUB.len() as u64 <= HIGH_RAM_START);
    }

    #[test]
    fn the_kernel_is_told_not_to_use_the_page() {
        let map = Ram::for_bytes(512 << 20).unwrap().e820();
        let range = map
            .iter()
            .find(|r| r.start <= RESET_VECTOR && RESET_VECTOR < r.start + r.bytes)
            .unwrap();
        assert_eq!(range.kind, Kind::Reserved);
    }
}
