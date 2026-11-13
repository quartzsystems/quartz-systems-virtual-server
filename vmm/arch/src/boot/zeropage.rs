// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The zero page: the kernel's own setup header given back to it, the
//! memory map, and where the ACPI root pointer is. There is no command
//! line to give: the kernel's is baked in and it ignores a loader's.

use linux_loader::configurator::linux::LinuxBootConfigurator;
use linux_loader::configurator::{BootConfigurator, BootParams};
use linux_loader::loader::bootparam::{boot_e820_entry, boot_params, setup_header};
use vm_memory::{Bytes, GuestAddress, GuestMemoryMmap};

use crate::Error;
use crate::layout::{ACPI_START, CMDLINE_START, Kind, Ram, ZERO_PAGE_START};

/// `HdrS`.
pub const HEADER_MAGIC: u32 = 0x5372_6448;
/// The boot sector's flag.
const BOOT_FLAG: u16 = 0xAA55;
/// A loader with no assigned id.
const LOADER_OTHER: u8 = 0xFF;
/// `xloadflags`: the kernel has a 64-bit entry at 0x200.
pub const XLF_KERNEL_64: u16 = 1;

const E820_RAM: u32 = 1;
const E820_RESERVED: u32 = 2;
const E820_ACPI: u32 = 3;

/// The boot parameters for a kernel whose setup header is `header`.
pub fn params(header: &setup_header, ram: &Ram) -> boot_params {
    let mut params = boot_params {
        hdr: *header,
        ..Default::default()
    };
    params.hdr.boot_flag = BOOT_FLAG;
    params.hdr.header = HEADER_MAGIC;
    params.hdr.type_of_loader = LOADER_OTHER;
    params.hdr.ramdisk_image = 0;
    params.hdr.ramdisk_size = 0;
    #[allow(clippy::cast_possible_truncation)] // below one megabyte
    {
        params.hdr.cmd_line_ptr = CMDLINE_START as u32;
    }
    params.acpi_rsdp_addr = ACPI_START;
    let map = ram.e820();
    for (slot, range) in params.e820_table.iter_mut().zip(&map) {
        *slot = boot_e820_entry {
            addr: range.start,
            size: range.bytes,
            r#type: match range.kind {
                Kind::Ram => E820_RAM,
                Kind::Reserved => E820_RESERVED,
                Kind::Acpi => E820_ACPI,
            },
        };
    }
    #[allow(clippy::cast_possible_truncation)] // seven entries at most
    {
        params.e820_entries = map.len() as u8;
    }
    params
}

/// Write the zero page and the empty command line.
pub fn write(mem: &GuestMemoryMmap, header: &setup_header, ram: &Ram) -> Result<(), Error> {
    mem.write_obj(0u8, GuestAddress(CMDLINE_START))
        .map_err(|_| Error::GuestMemory("the command line"))?;
    let params = params(header, ram);
    let boot = BootParams::new::<boot_params>(&params, GuestAddress(ZERO_PAGE_START));
    LinuxBootConfigurator::write_bootparams(&boot, mem)
        .map_err(|_| Error::GuestMemory("the zero page"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zero_page_names_the_memory_and_the_root_pointer() {
        let ram = Ram::for_bytes(8 << 30).unwrap();
        let header = setup_header {
            version: 0x20F,
            init_size: 0x300_0000,
            ..Default::default()
        };
        let p = params(&header, &ram);
        assert_eq!({ p.hdr.init_size }, 0x300_0000);
        assert_eq!({ p.hdr.type_of_loader }, 0xFF);
        assert_eq!({ p.acpi_rsdp_addr }, ACPI_START);
        assert_eq!(p.e820_entries, 8);
        let last = p.e820_table[7];
        assert_eq!(
            ({ last.addr }, { last.size }, { last.r#type }),
            (1 << 32, 5 << 30, 1)
        );
        let acpi = p.e820_table[2];
        assert_eq!(({ acpi.addr }, { acpi.r#type }), (ACPI_START, 3));
    }

    #[test]
    fn it_is_written_where_rsi_points() {
        let mem = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(0), 0x10_0000)]).unwrap();
        let ram = Ram::for_bytes(128 << 20).unwrap();
        write(&mem, &setup_header::default(), &ram).unwrap();
        // The header's magic is at 0x202 of the page.
        let magic: u32 = mem.read_obj(GuestAddress(ZERO_PAGE_START + 0x202)).unwrap();
        assert_eq!(magic, HEADER_MAGIC);
        let entries: u8 = mem.read_obj(GuestAddress(ZERO_PAGE_START + 0x1E8)).unwrap();
        assert_eq!(entries, 7);
    }
}
