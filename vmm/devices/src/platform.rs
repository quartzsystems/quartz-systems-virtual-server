// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a device asks of the hypervisor. The VMM answers with KVM;
//! a test answers with a record of what was asked.

use std::io;

use vmm_sys_util::eventfd::EventFd;

/// Where a message interrupt goes: the address and data the guest
/// wrote into an MSI-X table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsiRoute {
    /// The message address.
    pub address: u64,
    /// The message data.
    pub data: u32,
}

/// The hypervisor, as the devices see it.
pub trait Platform: Send + Sync {
    /// Raise interrupt `gsi` whenever `fd` is written.
    fn irqfd(&self, gsi: u32, fd: &EventFd) -> io::Result<()>;
    /// A new interrupt number for a message interrupt; `None` when
    /// there are no more.
    fn allocate_gsi(&self) -> Option<u32>;
    /// Route `gsi` to a message, or take its route away.
    fn route_msi(&self, gsi: u32, route: Option<MsiRoute>) -> io::Result<()>;
    /// Write `fd` whenever the guest writes at `address`, without the
    /// write leaving the kernel.
    fn ioeventfd(&self, address: u64, fd: &EventFd) -> io::Result<()>;
    /// Take that back.
    fn remove_ioeventfd(&self, address: u64, fd: &EventFd) -> io::Result<()>;
}

/// Why a device ends the partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// The guest powered off.
    PowerOff,
    /// The guest asked for a restart.
    Restart,
}

/// Where a device says so.
pub type Requests = Box<dyn Fn(Request) + Send + Sync>;

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// A platform that records.
    #[derive(Default)]
    pub struct Recorder {
        pub next_gsi: Mutex<u32>,
        pub routes: Mutex<Vec<(u32, Option<MsiRoute>)>>,
        pub ioevents: Mutex<Vec<u64>>,
    }

    impl Platform for Recorder {
        fn irqfd(&self, _: u32, _: &EventFd) -> io::Result<()> {
            Ok(())
        }
        fn allocate_gsi(&self) -> Option<u32> {
            let mut next = self.next_gsi.lock().unwrap();
            *next += 1;
            Some(23 + *next)
        }
        fn route_msi(&self, gsi: u32, route: Option<MsiRoute>) -> io::Result<()> {
            self.routes.lock().unwrap().push((gsi, route));
            Ok(())
        }
        fn ioeventfd(&self, address: u64, _: &EventFd) -> io::Result<()> {
            self.ioevents.lock().unwrap().push(address);
            Ok(())
        }
        fn remove_ioeventfd(&self, address: u64, _: &EventFd) -> io::Result<()> {
            self.ioevents.lock().unwrap().retain(|a| *a != address);
            Ok(())
        }
    }
}
