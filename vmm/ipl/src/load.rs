// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Loading a side's UKI: it is a bzImage, loaded with `linux-loader`
//! at a fixed address and entered at its 64-bit entry. The image is a
//! file from the guest's disk, so its header is checked here before
//! the loader sees it.

use std::io::Cursor;

use arch::boot::zeropage::{HEADER_MAGIC, XLF_KERNEL_64};
use arch::boot::{ENTRY_OFFSET, tables, zeropage};
use arch::layout::{HIGH_RAM_START, KERNEL_START, Ram};
use linux_loader::loader::{BzImage, KernelLoader};
use vm_memory::{GuestAddress, GuestMemoryMmap};

/// The boot protocol that has `xloadflags` and `init_size`.
const MIN_VERSION: u16 = 0x020C;
/// The header's fields end here.
const HEADER_END: usize = 0x268;

/// Why an image was not loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// The file is not a bzImage this loader can start.
    #[error("not a bzImage: {0}")]
    NotKernel(&'static str),
    /// The partition's memory does not hold the kernel.
    #[error("the kernel needs more memory than the partition has")]
    TooLarge,
    /// Guest memory could not be written.
    #[error("guest memory: {0}")]
    Memory(&'static str),
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Check the image's setup header. The bytes the kernel needs from
/// its load address on.
pub fn check(image: &[u8]) -> Result<u64, LoadError> {
    if image.len() < HEADER_END {
        return Err(LoadError::NotKernel("too short"));
    }
    if u32_at(image, 0x202) != HEADER_MAGIC {
        return Err(LoadError::NotKernel("no setup header"));
    }
    if u16_at(image, 0x206) < MIN_VERSION {
        return Err(LoadError::NotKernel("boot protocol older than 2.12"));
    }
    if u16_at(image, 0x236) & XLF_KERNEL_64 == 0 {
        return Err(LoadError::NotKernel("no 64-bit entry"));
    }
    if u64::from(u32_at(image, 0x214)) < HIGH_RAM_START {
        return Err(LoadError::NotKernel("not loaded high"));
    }
    let setup_sectors = match image[0x1F1] {
        0 => 4,
        n => usize::from(n),
    };
    let setup_bytes = (setup_sectors + 1) * 512;
    if image.len() <= setup_bytes {
        return Err(LoadError::NotKernel("no kernel after the setup code"));
    }
    let alignment = u64::from(u32_at(image, 0x230));
    let relocatable = image[0x234] != 0;
    let preferred = u64::from(u32_at(image, 0x258)) | (u64::from(u32_at(image, 0x25C)) << 32);
    if relocatable {
        if alignment == 0 || !alignment.is_power_of_two() || !KERNEL_START.is_multiple_of(alignment)
        {
            return Err(LoadError::NotKernel("alignment"));
        }
    } else if preferred != KERNEL_START {
        return Err(LoadError::NotKernel("not relocatable"));
    }
    let payload = (image.len() - setup_bytes) as u64;
    Ok(u64::from(u32_at(image, 0x260)).max(payload))
}

/// Load `image` into `mem`, write the page tables, the GDT and the
/// zero page. The address to enter the kernel at.
pub fn load(mem: &GuestMemoryMmap, ram: &Ram, image: &[u8]) -> Result<u64, LoadError> {
    let needed = check(image)?;
    if KERNEL_START
        .checked_add(needed)
        .is_none_or(|end| end > ram.low)
    {
        return Err(LoadError::TooLarge);
    }
    let loaded = BzImage::load(
        mem,
        Some(GuestAddress(KERNEL_START)),
        &mut Cursor::new(image),
        Some(GuestAddress(HIGH_RAM_START)),
    )
    .map_err(|_| LoadError::NotKernel("the loader refused it"))?;
    let header = loaded
        .setup_header
        .ok_or(LoadError::NotKernel("no setup header"))?;
    tables::write_gdt(mem).map_err(|_| LoadError::Memory("the GDT"))?;
    tables::write_page_tables(mem).map_err(|_| LoadError::Memory("the page tables"))?;
    zeropage::write(mem, &header, ram).map_err(|_| LoadError::Memory("the zero page"))?;
    Ok(loaded.kernel_load.0 + ENTRY_OFFSET)
}

/// A file with a setup header and `payload` bytes after it that the
/// loader takes, for the tests and the fuzzer's corpus. It is no
/// kernel: nothing can run it.
pub fn build(payload: usize) -> Vec<u8> {
    let mut image = vec![0u8; 2 * 512 + payload];
    image[0x1F1] = 1;
    image[0x1FE..0x200].copy_from_slice(&0xAA55u16.to_le_bytes());
    image[0x202..0x206].copy_from_slice(&HEADER_MAGIC.to_le_bytes());
    image[0x206..0x208].copy_from_slice(&0x020Fu16.to_le_bytes());
    image[0x211] = 0x01; // loaded high
    image[0x214..0x218].copy_from_slice(&0x10_0000u32.to_le_bytes());
    image[0x230..0x234].copy_from_slice(&0x20_0000u32.to_le_bytes());
    image[0x234] = 1;
    image[0x236..0x238].copy_from_slice(&XLF_KERNEL_64.to_le_bytes());
    image[0x258..0x260].copy_from_slice(&KERNEL_START.to_le_bytes());
    let init = u32::try_from(payload)
        .unwrap_or(u32::MAX)
        .saturating_add(0x10_0000);
    image[0x260..0x264].copy_from_slice(&init.to_le_bytes());
    for (i, byte) in image[1024..].iter_mut().enumerate() {
        *byte = u8::try_from(i % 251).unwrap_or(0);
    }
    image
}

/// A file the loader takes whose entry is `code`: for a suite that
/// needs a guest it can say everything about.
pub fn build_with(code: &[u8]) -> Vec<u8> {
    let entry = usize::try_from(ENTRY_OFFSET).unwrap_or(0x200);
    let mut image = build((entry + code.len()).max(4096));
    image[1024 + entry..1024 + entry + code.len()].copy_from_slice(code);
    image
}

/// A guest that does nothing but run: its boot processor jumps to
/// itself for ever, with interrupts off, and its other processors are
/// never started. It is busy from its first instruction, which is what
/// a suite that measures processor time wants of a guest.
pub fn spinner() -> Vec<u8> {
    // `jmp $`.
    build_with(&[0xEB, 0xFE])
}

#[cfg(test)]
mod tests {
    use super::*;
    use arch::layout::{PML4_START, ZERO_PAGE_START};
    use vm_memory::Bytes;

    fn memory(bytes: u64) -> (GuestMemoryMmap, Ram) {
        let ram = Ram::for_bytes(bytes).unwrap();
        let regions: Vec<(GuestAddress, usize)> = ram
            .regions()
            .into_iter()
            .map(|(start, len)| (GuestAddress(start), usize::try_from(len).unwrap()))
            .collect();
        (GuestMemoryMmap::from_ranges(&regions).unwrap(), ram)
    }

    #[test]
    fn an_image_is_loaded_at_the_fixed_address_with_its_boot_structures() {
        let (mem, ram) = memory(128 << 20);
        let image = build(8192);
        let entry = load(&mem, &ram, &image).unwrap();
        assert_eq!(entry, KERNEL_START + 0x200);
        let mut loaded = [0u8; 16];
        mem.read_slice(&mut loaded, GuestAddress(KERNEL_START))
            .unwrap();
        assert_eq!(loaded, image[1024..1040]);
        let magic: u32 = mem.read_obj(GuestAddress(ZERO_PAGE_START + 0x202)).unwrap();
        assert_eq!(magic, HEADER_MAGIC);
        let pml4: u64 = mem.read_obj(GuestAddress(PML4_START)).unwrap();
        assert_ne!(pml4, 0);
    }

    #[test]
    fn the_spinner_is_a_kernel_whose_entry_jumps_to_itself() {
        let image = spinner();
        assert!(check(&image).is_ok());
        let (mem, ram) = memory(128 << 20);
        let entry = load(&mem, &ram, &image).unwrap();
        let mut code = [0u8; 2];
        mem.read_slice(&mut code, GuestAddress(entry)).unwrap();
        assert_eq!(code, [0xEB, 0xFE]);
    }

    #[test]
    fn what_is_not_a_kernel_is_refused_by_name() {
        let (mem, ram) = memory(128 << 20);
        let good = build(4096);
        let refused = |image: &[u8]| load(&mem, &ram, image).unwrap_err();
        assert_eq!(refused(&[]), LoadError::NotKernel("too short"));
        assert_eq!(
            refused(&[0u8; 4096]),
            LoadError::NotKernel("no setup header")
        );
        let mut old = good.clone();
        old[0x206..0x208].copy_from_slice(&0x0204u16.to_le_bytes());
        assert_eq!(
            refused(&old),
            LoadError::NotKernel("boot protocol older than 2.12")
        );
        let mut no64 = good.clone();
        no64[0x236] = 0;
        assert_eq!(refused(&no64), LoadError::NotKernel("no 64-bit entry"));
        let mut setup = good.clone();
        setup[0x1F1] = 200;
        assert_eq!(
            refused(&setup),
            LoadError::NotKernel("no kernel after the setup code")
        );
        let mut align = good.clone();
        align[0x230..0x234].copy_from_slice(&0x300_0000u32.to_le_bytes());
        assert_eq!(refused(&align), LoadError::NotKernel("alignment"));
        let mut fixed = good.clone();
        fixed[0x234] = 0;
        fixed[0x258] = 1;
        assert_eq!(refused(&fixed), LoadError::NotKernel("not relocatable"));
        // A kernel that says it needs more than the partition has.
        let mut large = good;
        large[0x260..0x264].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(refused(&large), LoadError::TooLarge);
    }
}
