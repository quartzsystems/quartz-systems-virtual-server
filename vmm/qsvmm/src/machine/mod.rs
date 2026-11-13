// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition's machine on KVM (`docs/VMM.md`): memory with the
//! 32-bit hole left open, the in-kernel interrupt controller and PIT,
//! the four legacy devices on the I/O ports, the PCI bus with
//! virtio-scsi and virtio-rng, and a thread for each processor.
//!
//! - [`platform`]: KVM's interrupt routing and eventfds for the devices.
//! - [`vcpu`]: a processor's boot state and its run loop.

pub mod platform;
pub mod vcpu;

use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex};

use arch::layout::{
    ACPI_START, CMOS_PORT, I8042_PORT, IDENTITY_MAP_START, PCI_CONFIG_PORT, PCI_MEM_START, Ram,
    SERIAL_IRQ, SERIAL_PORT, SLEEP_PORT, TSS_START,
};
use devices::Memory;
use devices::bus::Bus;
use devices::legacy::cmos::Cmos;
use devices::legacy::power::Power;
use devices::legacy::reset::Reset;
use devices::legacy::serial::{Output, Serial};
use devices::pci::PciBus;
use devices::platform::{Platform, Requests};
use devices::virtio::rng::Rng;
use devices::virtio::scsi::Scsi;
use devices::virtio::scsi::lun::Lun;
use devices::virtio::{VirtioDevice, VirtioPci};
use kvm_bindings::{KVM_PIT_SPEAKER_DUMMY, kvm_pit_config};
use kvm_ioctls::{Cap, Kvm, VmFd};
use vm_memory::mmap::MmapRegionBuilder;
use vm_memory::{Bytes, GuestAddress, GuestMemoryBackend, GuestMemoryRegion, GuestRegionMmap};
use vmm_sys_util::eventfd::EventFd;

use platform::KvmPlatform;

/// The KVM abilities a partition cannot do without.
const NEEDED: [(Cap, &str); 6] = [
    (Cap::Irqchip, "an in-kernel interrupt controller"),
    (Cap::Ioeventfd, "ioeventfd"),
    (Cap::Irqfd, "irqfd"),
    (Cap::IrqRouting, "interrupt routing"),
    (Cap::Pit2, "an in-kernel PIT"),
    (Cap::TscDeadlineTimer, "the TSC deadline timer"),
];

/// Bytes of PCI memory space each function is given for its BARs.
const FUNCTION_SPACE: u32 = 0x1_0000;
/// Where in that space the second BAR is.
const SECOND_BAR: u32 = 0x8000;

fn kvm_error(what: &'static str) -> impl Fn(kvm_ioctls::Error) -> io::Error {
    move |e| {
        io::Error::other(format!(
            "{what}: {}",
            io::Error::from_raw_os_error(e.errno())
        ))
    }
}

/// The hypervisor, opened: what is done before memory exists.
pub struct Hypervisor {
    /// `/dev/kvm`.
    pub kvm: Kvm,
    /// The VM.
    pub vm: Arc<VmFd>,
}

impl Hypervisor {
    /// Open KVM and make a VM with the interrupt controller and PIT
    /// in the kernel.
    pub fn open() -> io::Result<Self> {
        let kvm = Kvm::new().map_err(kvm_error("/dev/kvm"))?;
        for (cap, what) in NEEDED {
            if !kvm.check_extension(cap) {
                return Err(io::Error::other(format!("KVM lacks {what}")));
            }
        }
        let vm = kvm.create_vm().map_err(kvm_error("the VM"))?;
        #[allow(clippy::cast_possible_truncation)] // below 4 GiB
        vm.set_tss_address(TSS_START as usize)
            .map_err(kvm_error("the TSS address"))?;
        vm.set_identity_map_address(IDENTITY_MAP_START)
            .map_err(kvm_error("the identity map address"))?;
        vm.create_irq_chip()
            .map_err(kvm_error("the interrupt controller"))?;
        let pit = kvm_pit_config {
            flags: KVM_PIT_SPEAKER_DUMMY,
            ..Default::default()
        };
        vm.create_pit2(pit).map_err(kvm_error("the PIT"))?;
        Ok(Self {
            kvm,
            vm: Arc::new(vm),
        })
    }

    /// The most processors KVM gives a VM.
    pub fn max_vcpus(&self) -> usize {
        self.kvm.get_max_vcpus()
    }
}

/// The guest's memory: an anonymous shared mapping for each region,
/// given to KVM. With `huge`, of the 2 MiB pages the host reserved at
/// its IPL, and reserved for this mapping here: when they are not
/// there the mapping fails now, not the guest later.
pub fn memory(vm: &VmFd, ram: &Ram, huge: bool) -> io::Result<Arc<Memory>> {
    let flags = if huge {
        libc::MAP_ANONYMOUS | libc::MAP_SHARED | libc::MAP_HUGETLB | libc::MAP_HUGE_2MB
    } else {
        libc::MAP_ANONYMOUS | libc::MAP_SHARED | libc::MAP_NORESERVE
    };
    let mut regions = Vec::new();
    for (start, bytes) in ram.regions() {
        let bytes = usize::try_from(bytes).map_err(io::Error::other)?;
        let mapping = MmapRegionBuilder::new(bytes)
            .with_mmap_prot(libc::PROT_READ | libc::PROT_WRITE)
            .with_mmap_flags(flags)
            .with_hugetlbfs(huge)
            .build()
            .map_err(|e| io::Error::other(format!("guest memory: {e}")))?;
        let region = GuestRegionMmap::new(mapping, GuestAddress(start))
            .ok_or_else(|| io::Error::other("guest memory: the region's address"))?;
        regions.push(region);
    }
    let memory = Memory::from_regions(regions)
        .map_err(|e| io::Error::other(format!("guest memory: {e}")))?;
    for (slot, region) in (0u32..).zip(memory.iter()) {
        let host = region
            .get_host_address(vm_memory::MemoryRegionAddress(0))
            .map_err(|e| io::Error::other(format!("guest memory: {e}")))?;
        crate::sys::register_memory(vm, slot, region.start_addr().0, region.len(), host)?;
    }
    Ok(Arc::new(memory))
}

/// Write the ACPI tables for `processors` processors into memory.
pub fn write_acpi(memory: &Memory, processors: u32) -> io::Result<()> {
    let processors = u8::try_from(processors).map_err(io::Error::other)?;
    let tables = acpi::build(processors, ACPI_START)
        .ok_or_else(|| io::Error::other("the ACPI tables do not fit"))?;
    memory
        .write_slice(&tables.bytes, GuestAddress(ACPI_START))
        .map_err(|e| io::Error::other(format!("the ACPI tables: {e}")))?;
    arch::boot::reset::write(memory).map_err(io::Error::other)
}

/// The devices of a partition.
pub struct Devices {
    /// The I/O ports.
    pub io: Arc<Bus>,
    /// The PCI bus: memory accesses outside RAM go here.
    pub pci: Arc<Mutex<PciBus>>,
    /// The serial port, for the console's input.
    pub serial: Arc<Mutex<Serial>>,
    /// The disk LUNs of each adapter, for the counts at the end.
    pub luns: Vec<Arc<Vec<Lun>>>,
}

fn put(
    bus: &mut Bus,
    port: u16,
    len: u64,
    device: devices::bus::Shared,
    what: &str,
) -> io::Result<()> {
    if bus.insert(u64::from(port), len, device) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "the I/O ports of {what} are taken"
        )))
    }
}

/// Build the devices: the serial port writing to `output`, the clock,
/// the reset line and the power-off register telling `requests`, and
/// on the PCI bus a virtio-scsi function for each of `adapters`, with
/// its units, and after them virtio-rng reading `entropy`.
pub fn devices(
    vm: &Arc<VmFd>,
    memory: &Arc<Memory>,
    output: Output,
    requests: Requests,
    adapters: Vec<Vec<Lun>>,
    entropy: File,
) -> io::Result<Devices> {
    let platform: Arc<dyn Platform> = Arc::new(KvmPlatform::new(vm.clone()));
    let requests = Arc::new(requests);
    let mut io = Bus::new();

    let irq = EventFd::new(0)?;
    platform.irqfd(SERIAL_IRQ, &irq)?;
    let serial = Arc::new(Mutex::new(Serial::new(irq, output)));
    put(&mut io, SERIAL_PORT, 8, serial.clone(), "the serial port")?;
    put(
        &mut io,
        CMOS_PORT,
        2,
        Arc::new(Mutex::new(Cmos::new())),
        "the clock",
    )?;
    let reset = Arc::new(Mutex::new(Reset::new(requests.clone())));
    put(&mut io, I8042_PORT, 5, reset, "the reset line")?;
    let power = Arc::new(Mutex::new(Power::new(requests)));
    put(&mut io, SLEEP_PORT, 2, power, "the sleep register")?;

    let mut luns = Vec::with_capacity(adapters.len());
    let mut functions: Vec<Box<dyn VirtioDevice>> = Vec::with_capacity(adapters.len() + 1);
    for units in adapters {
        let scsi = Scsi::new(units).ok_or_else(|| io::Error::other("the number of disk units"))?;
        luns.push(scsi.luns());
        functions.push(Box::new(scsi));
    }
    functions.push(Box::new(Rng::new(entropy)));
    let mut pci = PciBus::new();
    #[allow(clippy::cast_possible_truncation)] // 3 GiB
    let mut base = PCI_MEM_START as u32;
    for function in functions {
        let function = VirtioPci::new(
            function,
            platform.clone(),
            memory.clone(),
            base,
            base + SECOND_BAR,
        )?;
        pci.add(Arc::new(Mutex::new(function)))
            .ok_or_else(|| io::Error::other("the PCI bus is full"))?;
        base += FUNCTION_SPACE;
    }
    let pci = Arc::new(Mutex::new(pci));
    put(
        &mut io,
        PCI_CONFIG_PORT,
        8,
        pci.clone(),
        "PCI configuration",
    )?;

    Ok(Devices {
        io: Arc::new(io),
        pci,
        serial,
        luns,
    })
}
