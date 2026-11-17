// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition's physical I/O (`docs/PHYSICALIO.md`), from the
//! descriptors the partition manager opened: the VFIO container and
//! each isolation group. Nothing is opened by path. The groups are put
//! in the container, which gets the IOMMU; KVM's VFIO device is told of
//! them; each function's descriptor is taken from its group. Then the
//! partition's memory, and nothing else, is mapped for the devices'
//! DMA, at the guest's own addresses: a device reaches its partition's
//! memory only.

use std::io;
use std::path::PathBuf;

use devices::vfio::{Container, Device, Group};
use kvm_bindings::{KVM_DEV_VFIO_FILE, KVM_DEV_VFIO_FILE_ADD, kvm_create_device, kvm_device_attr};
use kvm_ioctls::{DeviceFd, VmFd};
use vm_memory::{GuestMemoryBackend, GuestMemoryRegion};

use crate::config::{self, Partition};

/// The partition's devices, opened.
pub struct Physio {
    container: Container,
    /// Each group, by the descriptor it was given as.
    groups: Vec<(PathBuf, Group)>,
    /// Each function with what the configuration says of it.
    pub devices: Vec<(config::Device, Device)>,
    /// KVM's VFIO device, kept while the VM lives.
    _kvm: DeviceFd,
}

fn other(what: String) -> io::Error {
    io::Error::other(what)
}

/// Open what the partition was given; none without devices.
pub fn open(partition: &Partition, vm: &VmFd) -> io::Result<Option<Physio>> {
    let config = &partition.config;
    let Some(path) = &config.vfio else {
        return Ok(None);
    };
    let fd = crate::net::descriptor(path)
        .ok_or_else(|| other(format!("{} is no descriptor", path.display())))?;
    let container = Container::new(crate::sys::inherited(fd)?)?;
    let mut groups: Vec<(PathBuf, Group)> = Vec::new();
    for d in &config.devices {
        if groups.iter().any(|(p, _)| *p == d.group) {
            continue;
        }
        let fd = crate::net::descriptor(&d.group)
            .ok_or_else(|| other(format!("{}: its group is no descriptor", d.location)))?;
        let group = Group(crate::sys::inherited(fd)?);
        if !group.viable()? {
            return Err(other(format!(
                "{}: its isolation group has a device the host's drivers hold",
                d.location
            )));
        }
        container.attach(&group)?;
        groups.push((d.group.clone(), group));
    }
    container.set_iommu()?;
    let kvm = kvm_vfio(vm, &groups)?;
    let mut devices = Vec::with_capacity(config.devices.len());
    for d in &config.devices {
        let (_, group) = groups
            .iter()
            .find(|(p, _)| *p == d.group)
            .ok_or_else(|| other(format!("{}: no group", d.location)))?;
        let device = group
            .device(&d.address)
            .map_err(|e| other(format!("{} ({}): {e}", d.location, d.address)))?;
        devices.push((d.clone(), device));
    }
    Ok(Some(Physio {
        container,
        groups,
        devices,
        _kvm: kvm,
    }))
}

/// KVM's VFIO device, with every group added: KVM then knows that the
/// guest has devices that can DMA, and joins their interrupts to the
/// guest's without the VMM.
fn kvm_vfio(vm: &VmFd, groups: &[(PathBuf, Group)]) -> io::Result<DeviceFd> {
    let mut create = kvm_create_device {
        type_: kvm_bindings::kvm_device_type_KVM_DEV_TYPE_VFIO,
        fd: 0,
        flags: 0,
    };
    let device = vm
        .create_device(&mut create)
        .map_err(|e| other(format!("KVM's VFIO device: {e}")))?;
    for (_, group) in groups {
        let fd: i32 = std::os::fd::AsRawFd::as_raw_fd(&group.0);
        let attr = kvm_device_attr {
            flags: 0,
            group: KVM_DEV_VFIO_FILE,
            attr: u64::from(KVM_DEV_VFIO_FILE_ADD),
            addr: (&raw const fd) as u64,
        };
        device
            .set_device_attr(&attr)
            .map_err(|e| other(format!("KVM's VFIO device, a group: {e}")))?;
    }
    Ok(device)
}

impl Physio {
    /// Map the partition's memory, every region of it at its guest
    /// address, for the devices' DMA. The pages are pinned from now on.
    pub fn map_memory(&self, memory: &devices::Memory) -> io::Result<u64> {
        let mut bytes = 0;
        for region in memory.iter() {
            let host = region
                .get_host_address(vm_memory::MemoryRegionAddress(0))
                .map_err(|e| other(format!("guest memory: {e}")))?;
            self.container
                .map(region.start_addr().0, region.len(), host as u64)
                .map_err(|e| other(format!("mapping memory for DMA: {e}")))?;
            bytes += region.len();
        }
        Ok(bytes)
    }

    /// How many groups the partition has.
    pub fn groups(&self) -> usize {
        self.groups.len()
    }
}
