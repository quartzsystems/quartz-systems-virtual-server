// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! VFIO (`linux/vfio.h`), on descriptors the partition manager opened:
//! the container that is the partition's IOMMU domain, its isolation
//! groups, and each device. The structures are rust-vmm's
//! `vfio-bindings`; the calls are here, because `vfio-ioctls` opens
//! `/dev/vfio/vfio` and the groups by path itself, which an
//! unprivileged VMM cannot do. Each call passes a struct of the layout
//! the kernel reads, by reference, for the length of the call.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;

use vfio_bindings::bindings::vfio::{
    __IncompleteArrayField, VFIO_API_VERSION, VFIO_DMA_MAP_FLAG_READ, VFIO_DMA_MAP_FLAG_WRITE,
    VFIO_GROUP_FLAGS_VIABLE, VFIO_IRQ_SET_ACTION_TRIGGER, VFIO_IRQ_SET_DATA_EVENTFD,
    VFIO_IRQ_SET_DATA_NONE, VFIO_PCI_CONFIG_REGION_INDEX, VFIO_PCI_MSI_IRQ_INDEX,
    VFIO_PCI_MSIX_IRQ_INDEX, VFIO_REGION_INFO_FLAG_MMAP, VFIO_TYPE1v2_IOMMU, vfio_device_info,
    vfio_group_status, vfio_iommu_type1_dma_map, vfio_irq_set, vfio_region_info,
};
use vm_memory::FileOffset;
use vm_memory::mmap::{MmapRegion, MmapRegionBuilder};
use vmm_sys_util::eventfd::EventFd;
use vmm_sys_util::ioctl::{ioctl, ioctl_with_mut_ref, ioctl_with_ref, ioctl_with_val};

use crate::pci::passthrough::{Function, Interrupts, Region};

/// The calls' numbers: `_IO(VFIO_TYPE, VFIO_BASE + n)`, VFIO's calls
/// carrying no size.
mod nr {
    use vmm_sys_util::ioctl_io_nr;

    ioctl_io_nr!(VFIO_GET_API_VERSION, 59, 100);
    ioctl_io_nr!(VFIO_CHECK_EXTENSION, 59, 101);
    ioctl_io_nr!(VFIO_SET_IOMMU, 59, 102);
    ioctl_io_nr!(VFIO_GROUP_GET_STATUS, 59, 103);
    ioctl_io_nr!(VFIO_GROUP_SET_CONTAINER, 59, 104);
    ioctl_io_nr!(VFIO_GROUP_GET_DEVICE_FD, 59, 106);
    ioctl_io_nr!(VFIO_DEVICE_GET_INFO, 59, 107);
    ioctl_io_nr!(VFIO_DEVICE_GET_REGION_INFO, 59, 108);
    ioctl_io_nr!(VFIO_DEVICE_SET_IRQS, 59, 110);
    ioctl_io_nr!(VFIO_IOMMU_MAP_DMA, 59, 113);
}

use nr::{
    VFIO_CHECK_EXTENSION, VFIO_DEVICE_GET_INFO, VFIO_DEVICE_GET_REGION_INFO, VFIO_DEVICE_SET_IRQS,
    VFIO_GET_API_VERSION, VFIO_GROUP_GET_DEVICE_FD, VFIO_GROUP_GET_STATUS,
    VFIO_GROUP_SET_CONTAINER, VFIO_IOMMU_MAP_DMA, VFIO_SET_IOMMU,
};

/// The most vectors one call sets.
const MAX_FDS: usize = 64;

fn check(result: i32) -> io::Result<i32> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn argsz<T>() -> u32 {
    u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX)
}

/// The partition's container: one IOMMU domain for all its groups.
pub struct Container(File);

impl Container {
    /// The container the partition manager opened (`/dev/vfio/vfio`),
    /// checked to speak this API with the type 1 IOMMU.
    pub fn new(file: File) -> io::Result<Self> {
        // SAFETY: GET_API_VERSION takes no argument and touches no
        // memory of this process.
        let version = check(unsafe { ioctl(&file, VFIO_GET_API_VERSION()) })?;
        if u32::try_from(version).ok() != Some(VFIO_API_VERSION) {
            return Err(io::Error::other(format!("VFIO API version {version}")));
        }
        // SAFETY: CHECK_EXTENSION takes its argument by value.
        let type1 = check(unsafe {
            ioctl_with_val(&file, VFIO_CHECK_EXTENSION(), VFIO_TYPE1v2_IOMMU.into())
        })?;
        if type1 != 1 {
            return Err(io::Error::other("the IOMMU has no type 1 (v2) VFIO driver"));
        }
        Ok(Self(file))
    }

    /// Put `group` in this container.
    pub fn attach(&self, group: &Group) -> io::Result<()> {
        let fd: i32 = self.0.as_raw_fd();
        // SAFETY: SET_CONTAINER reads one `int`, the container's
        // descriptor, from the reference for the length of the call.
        check(unsafe { ioctl_with_ref(&group.0, VFIO_GROUP_SET_CONTAINER(), &fd) })?;
        Ok(())
    }

    /// Give the container its IOMMU, once its first group is in it.
    pub fn set_iommu(&self) -> io::Result<()> {
        // SAFETY: SET_IOMMU takes its argument by value.
        check(unsafe { ioctl_with_val(&self.0, VFIO_SET_IOMMU(), VFIO_TYPE1v2_IOMMU.into()) })?;
        Ok(())
    }

    /// Map `size` bytes of this process's memory at `vaddr` for the
    /// devices' DMA at `iova`, readable and writable. The pages are
    /// pinned for as long as the container is open.
    pub fn map(&self, iova: u64, size: u64, vaddr: u64) -> io::Result<()> {
        let map = vfio_iommu_type1_dma_map {
            argsz: argsz::<vfio_iommu_type1_dma_map>(),
            flags: VFIO_DMA_MAP_FLAG_READ | VFIO_DMA_MAP_FLAG_WRITE,
            vaddr,
            iova,
            size,
        };
        // SAFETY: MAP_DMA reads the struct, of the size its `argsz`
        // says, for the length of the call; the memory it names is the
        // guest's, which the machine keeps mapped while the container
        // is open.
        check(unsafe { ioctl_with_ref(&self.0, VFIO_IOMMU_MAP_DMA(), &map) })?;
        Ok(())
    }
}

/// One isolation group the partition manager opened (`/dev/vfio/<n>`).
pub struct Group(pub File);

impl Group {
    /// Whether every device of the group is bound to VFIO or no driver.
    pub fn viable(&self) -> io::Result<bool> {
        let mut status = vfio_group_status {
            argsz: argsz::<vfio_group_status>(),
            flags: 0,
        };
        // SAFETY: GET_STATUS writes the struct it is given, of the size
        // its `argsz` says, and nothing else.
        check(unsafe { ioctl_with_mut_ref(&self.0, VFIO_GROUP_GET_STATUS(), &mut status) })?;
        Ok(status.flags & VFIO_GROUP_FLAGS_VIABLE != 0)
    }

    /// The device at the PCI address `address` (`0000:00:06.0`).
    pub fn device(&self, address: &str) -> io::Result<Device> {
        if address.len() > 32
            || !address
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
        {
            return Err(io::Error::other(format!(
                "{address:?} is not a PCI address"
            )));
        }
        let mut name = address.as_bytes().to_vec();
        name.push(0);
        // SAFETY: GET_DEVICE_FD reads the NUL-terminated name it is
        // given and returns a new descriptor, which is owned here.
        let fd = check(unsafe {
            vmm_sys_util::ioctl::ioctl_with_ptr(&self.0, VFIO_GROUP_GET_DEVICE_FD(), name.as_ptr())
        })?;
        // SAFETY: the descriptor was just returned and nothing else
        // owns it.
        let file = unsafe { <File as std::os::fd::FromRawFd>::from_raw_fd(fd) };
        Device::new(file)
    }
}

/// A region of a device: where it is in the device's file.
#[derive(Clone, Copy, Debug, Default)]
struct Area {
    offset: u64,
    size: u64,
    flags: u32,
}

/// One device, reached through its VFIO descriptor.
pub struct Device {
    file: File,
    areas: [Area; 8],
}

impl Device {
    fn new(file: File) -> io::Result<Self> {
        let mut info = vfio_device_info {
            argsz: argsz::<vfio_device_info>(),
            ..Default::default()
        };
        // SAFETY: GET_INFO writes the struct it is given, of the size
        // its `argsz` says.
        check(unsafe { ioctl_with_mut_ref(&file, VFIO_DEVICE_GET_INFO(), &mut info) })?;
        let mut areas = [Area::default(); 8];
        let count = info.num_regions.min(8);
        for index in 0..count {
            let mut r = vfio_region_info {
                argsz: argsz::<vfio_region_info>(),
                index,
                ..Default::default()
            };
            // SAFETY: GET_REGION_INFO writes the struct it is given, of
            // the size its `argsz` says; capabilities past it are not
            // asked for.
            check(unsafe { ioctl_with_mut_ref(&file, VFIO_DEVICE_GET_REGION_INFO(), &mut r) })?;
            areas[index as usize] = Area {
                offset: r.offset,
                size: r.size,
                flags: r.flags,
            };
        }
        if areas[VFIO_PCI_CONFIG_REGION_INDEX as usize].size == 0 {
            return Err(io::Error::other("the device has no configuration space"));
        }
        Ok(Self { file, areas })
    }

    /// The same device on a second descriptor: one for the bus, one to
    /// map its BARs with.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            areas: self.areas,
        })
    }

    /// Its six BARs' regions.
    pub fn regions(&self) -> [Region; 6] {
        let mut out = [Region::default(); 6];
        for (o, a) in out.iter_mut().zip(&self.areas) {
            *o = Region {
                size: a.size,
                mappable: a.flags & VFIO_REGION_INFO_FLAG_MMAP != 0,
            };
        }
        out
    }

    /// Map `len` bytes of BAR `bar` from `offset` into this process.
    pub fn map(&self, bar: usize, offset: u64, len: u64) -> io::Result<MmapRegion> {
        let a = self
            .areas
            .get(bar)
            .filter(|a| bar < 6 && a.flags & VFIO_REGION_INFO_FLAG_MMAP != 0)
            .ok_or_else(|| io::Error::other(format!("BAR {bar} cannot be mapped")))?;
        if offset.checked_add(len).is_none_or(|end| end > a.size) {
            return Err(io::Error::other(format!("BAR {bar}: past its end")));
        }
        let start = a.offset + offset;
        let len = usize::try_from(len).map_err(io::Error::other)?;
        MmapRegionBuilder::new(len)
            .with_file_offset(FileOffset::new(self.file.try_clone()?, start))
            .with_mmap_prot(libc::PROT_READ | libc::PROT_WRITE)
            .with_mmap_flags(libc::MAP_SHARED)
            .build()
            .map_err(|e| io::Error::other(format!("BAR {bar}: {e}")))
    }

    fn read(&self, area: usize, offset: u64, data: &mut [u8]) {
        let ok = self.areas.get(area).is_some_and(|a| {
            offset
                .checked_add(data.len() as u64)
                .is_some_and(|end| end <= a.size)
                && self.file.read_exact_at(data, a.offset + offset).is_ok()
        });
        if !ok {
            data.fill(0xFF);
        }
    }

    fn write(&self, area: usize, offset: u64, data: &[u8]) {
        if let Some(a) = self.areas.get(area)
            && offset
                .checked_add(data.len() as u64)
                .is_some_and(|end| end <= a.size)
        {
            let _ = self.file.write_all_at(data, a.offset + offset);
        }
    }
}

/// `struct vfio_irq_set` with room for its descriptors.
#[repr(C)]
struct IrqSet {
    head: vfio_irq_set,
    fds: [i32; MAX_FDS],
}

impl Function for Device {
    fn config_read(&self, offset: u16, data: &mut [u8]) {
        self.read(VFIO_PCI_CONFIG_REGION_INDEX as usize, offset.into(), data);
    }

    fn config_write(&self, offset: u16, data: &[u8]) {
        self.write(VFIO_PCI_CONFIG_REGION_INDEX as usize, offset.into(), data);
    }

    fn bar_read(&self, bar: usize, offset: u64, data: &mut [u8]) {
        if bar < 6 {
            self.read(bar, offset, data);
        } else {
            data.fill(0xFF);
        }
    }

    fn bar_write(&self, bar: usize, offset: u64, data: &[u8]) {
        if bar < 6 {
            self.write(bar, offset, data);
        }
    }

    fn interrupts(&self, kind: Interrupts, fds: &[&EventFd]) -> io::Result<()> {
        let index = match kind {
            Interrupts::Msi => VFIO_PCI_MSI_IRQ_INDEX,
            Interrupts::Msix => VFIO_PCI_MSIX_IRQ_INDEX,
        };
        if fds.len() > MAX_FDS {
            return Err(io::Error::other("too many vectors"));
        }
        let count = u32::try_from(fds.len()).unwrap_or(0);
        let mut set = IrqSet {
            head: vfio_irq_set {
                argsz: argsz::<vfio_irq_set>() + count * 4,
                flags: VFIO_IRQ_SET_ACTION_TRIGGER
                    | if fds.is_empty() {
                        VFIO_IRQ_SET_DATA_NONE
                    } else {
                        VFIO_IRQ_SET_DATA_EVENTFD
                    },
                index,
                start: 0,
                count,
                data: __IncompleteArrayField::new(),
            },
            fds: [-1; MAX_FDS],
        };
        for (slot, fd) in set.fds.iter_mut().zip(fds) {
            *slot = fd.as_raw_fd();
        }
        // SAFETY: SET_IRQS reads the header and `count` descriptors
        // after it, which `argsz` covers and the struct holds, for the
        // length of the call.
        check(unsafe { ioctl_with_ref(&self.file, VFIO_DEVICE_SET_IRQS(), &set) })?;
        Ok(())
    }
}
