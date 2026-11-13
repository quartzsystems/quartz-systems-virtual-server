// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the devices ask of the hypervisor, answered with KVM: an
//! interrupt line raised from an eventfd, a message interrupt's
//! route, a notification delivered by eventfd.
//!
//! KVM has one routing table for every interrupt number. The first 24
//! go to the interrupt controllers' pins as KVM sets them up itself;
//! message interrupts are numbered from 24 and each has the route its
//! MSI-X table entry gives. A change writes the whole table again.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use arch::layout::FIRST_MSI_GSI;
use devices::platform::{MsiRoute, Platform};
use kvm_bindings::{
    KVM_IRQ_ROUTING_IRQCHIP, KVM_IRQ_ROUTING_MSI, KVM_IRQCHIP_IOAPIC, KVM_IRQCHIP_PIC_MASTER,
    KVM_IRQCHIP_PIC_SLAVE, KvmIrqRouting, kvm_irq_routing_entry, kvm_irq_routing_irqchip,
    kvm_irq_routing_msi,
};
use kvm_ioctls::{IoEventAddress, NoDatamatch, VmFd};
use vmm_sys_util::eventfd::EventFd;

/// Interrupt numbers a partition may use: more than its devices'
/// vectors, fewer than KVM's table holds.
const GSI_LIMIT: u32 = 1024;
/// Pins of the I/O APIC.
const IOAPIC_PINS: u32 = 24;

fn kvm_error(e: kvm_ioctls::Error) -> io::Error {
    io::Error::from_raw_os_error(e.errno())
}

fn pin(gsi: u32, irqchip: u32, pin: u32) -> kvm_irq_routing_entry {
    let mut entry = kvm_irq_routing_entry {
        gsi,
        type_: KVM_IRQ_ROUTING_IRQCHIP,
        ..Default::default()
    };
    entry.u.irqchip = kvm_irq_routing_irqchip { irqchip, pin };
    entry
}

/// The routes of the interrupt controllers' pins, as KVM has them
/// before any other is set.
pub fn default_routes() -> Vec<kvm_irq_routing_entry> {
    let mut routes = Vec::new();
    for gsi in 0..8 {
        routes.push(pin(gsi, KVM_IRQCHIP_PIC_MASTER, gsi));
    }
    for gsi in 8..16 {
        routes.push(pin(gsi, KVM_IRQCHIP_PIC_SLAVE, gsi - 8));
    }
    for gsi in 0..IOAPIC_PINS {
        routes.push(pin(gsi, KVM_IRQCHIP_IOAPIC, gsi));
    }
    routes
}

fn message(gsi: u32, route: MsiRoute) -> kvm_irq_routing_entry {
    let mut entry = kvm_irq_routing_entry {
        gsi,
        type_: KVM_IRQ_ROUTING_MSI,
        ..Default::default()
    };
    #[allow(clippy::cast_possible_truncation)] // the two halves
    {
        entry.u.msi = kvm_irq_routing_msi {
            address_lo: route.address as u32,
            address_hi: (route.address >> 32) as u32,
            data: route.data,
            ..Default::default()
        };
    }
    entry
}

struct Routes {
    next_gsi: u32,
    messages: BTreeMap<u32, MsiRoute>,
}

/// KVM, as the devices see it.
pub struct KvmPlatform {
    vm: Arc<VmFd>,
    routes: Mutex<Routes>,
}

impl KvmPlatform {
    /// The platform of `vm`.
    pub fn new(vm: Arc<VmFd>) -> Self {
        Self {
            vm,
            routes: Mutex::new(Routes {
                next_gsi: FIRST_MSI_GSI,
                messages: BTreeMap::new(),
            }),
        }
    }
}

impl Platform for KvmPlatform {
    fn irqfd(&self, gsi: u32, fd: &EventFd) -> io::Result<()> {
        self.vm.register_irqfd(fd, gsi).map_err(kvm_error)
    }

    fn allocate_gsi(&self) -> Option<u32> {
        let mut routes = self.routes.lock().ok()?;
        let gsi = routes.next_gsi;
        (gsi < GSI_LIMIT).then(|| {
            routes.next_gsi += 1;
            gsi
        })
    }

    fn route_msi(&self, gsi: u32, route: Option<MsiRoute>) -> io::Result<()> {
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?;
        if !(FIRST_MSI_GSI..routes.next_gsi).contains(&gsi) {
            return Err(io::Error::other("not a message interrupt"));
        }
        match route {
            Some(route) => routes.messages.insert(gsi, route),
            None => routes.messages.remove(&gsi),
        };
        let mut entries = default_routes();
        entries.extend(
            routes
                .messages
                .iter()
                .map(|(gsi, route)| message(*gsi, *route)),
        );
        let table =
            KvmIrqRouting::from_entries(&entries).map_err(|_| io::Error::other("routing table"))?;
        self.vm.set_gsi_routing(&table).map_err(kvm_error)
    }

    fn ioeventfd(&self, address: u64, fd: &EventFd) -> io::Result<()> {
        self.vm
            .register_ioevent(fd, &IoEventAddress::Mmio(address), NoDatamatch)
            .map_err(kvm_error)
    }

    fn remove_ioeventfd(&self, address: u64, fd: &EventFd) -> io::Result<()> {
        self.vm
            .unregister_ioevent(fd, &IoEventAddress::Mmio(address), NoDatamatch)
            .map_err(kvm_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pins_are_routed_as_kvm_routes_them() {
        let routes = default_routes();
        assert_eq!(routes.len(), 16 + 24);
        // Interrupt 4, the serial port's: the first controller's pin 4
        // and the I/O APIC's pin 4.
        let four: Vec<_> = routes.iter().filter(|r| r.gsi == 4).collect();
        assert_eq!(four.len(), 2);
        assert!(routes.iter().all(|r| r.type_ == KVM_IRQ_ROUTING_IRQCHIP));
        assert!(routes.iter().all(|r| r.gsi < FIRST_MSI_GSI));
        let m = message(
            24,
            MsiRoute {
                address: 0x1_FEE0_1000,
                data: 0x4041,
            },
        );
        assert_eq!((m.gsi, m.type_), (24, KVM_IRQ_ROUTING_MSI));
    }
}
