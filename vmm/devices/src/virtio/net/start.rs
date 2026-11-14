// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! vhost-net started on the adapter's receive and transmit queues: its
//! owner and the guest's memory once, then at each driver's start the
//! features, the tap's offloads, and each queue's size, base, rings,
//! notification eventfd (the transport's ioeventfd), interrupt eventfd
//! (the vector's irqfd), and last the tap.

use std::fs::File;
use std::io;

use virtio_queue::QueueT;
use vm_memory::{Address, GuestAddress, GuestMemoryBackend, GuestMemoryRegion};

use super::{
    ActiveQueue, F_GUEST_CSUM, F_GUEST_ECN, F_GUEST_TSO4, F_GUEST_TSO6, F_MRG_RXBUF, Interrupt, Net,
};
use crate::sys::vhost::{self, MAX_REGIONS, MemRegion, MemTable, VringAddr};
use crate::virtio::{FEATURE_INDIRECT_DESC, FEATURE_VERSION_1};

/// What vhost-net takes of the features a driver accepted: the rest are
/// the tap's (offloads) or the VMM's (the control queue).
const VHOST_TAKES: u64 = F_MRG_RXBUF | FEATURE_VERSION_1 | FEATURE_INDIRECT_DESC;

/// `TUN_F_CSUM`, `TUN_F_TSO4`, `TUN_F_TSO6`, `TUN_F_TSO_ECN`.
const TUN_F_CSUM: u32 = 0x01;
const TUN_F_TSO4: u32 = 0x02;
const TUN_F_TSO6: u32 = 0x04;
const TUN_F_TSO_ECN: u32 = 0x08;

/// What the tap may give the guest, by what the driver accepted.
fn offloads(features: u64) -> u32 {
    let mut flags = 0;
    if features & F_GUEST_CSUM != 0 {
        flags |= TUN_F_CSUM;
        if features & F_GUEST_TSO4 != 0 {
            flags |= TUN_F_TSO4;
        }
        if features & F_GUEST_TSO6 != 0 {
            flags |= TUN_F_TSO6;
        }
        if features & F_GUEST_ECN != 0 && flags & (TUN_F_TSO4 | TUN_F_TSO6) != 0 {
            flags |= TUN_F_TSO_ECN;
        }
    }
    flags
}

/// The VMM's address of guest address `at`.
fn host(memory: &crate::Memory, at: u64) -> io::Result<u64> {
    let p = memory
        .get_host_address(GuestAddress(at))
        .map_err(io::Error::other)?;
    u64::try_from(p.addr()).map_err(io::Error::other)
}

/// The guest's memory as vhost-net is given it.
fn table(memory: &crate::Memory) -> io::Result<MemTable> {
    let mut table = MemTable::default();
    for (i, region) in memory.iter().enumerate() {
        let slot = table
            .regions
            .get_mut(i)
            .ok_or_else(|| io::Error::other(format!("more than {MAX_REGIONS} memory regions")))?;
        *slot = MemRegion {
            guest: region.start_addr().raw_value(),
            size: region.len(),
            user: host(memory, region.start_addr().raw_value())?,
            padding: 0,
        };
        table.count = u32::try_from(i + 1).unwrap_or(u32::MAX);
    }
    Ok(table)
}

impl Net {
    /// Start vhost-net on `queues`, the receive and the transmit queue,
    /// for a driver that accepted `features`.
    pub(super) fn start(
        &mut self,
        memory: &crate::Memory,
        interrupt: &Interrupt,
        features: u64,
        queues: &[ActiveQueue; 2],
    ) -> io::Result<()> {
        let Some(b) = &self.backend else {
            return Err(io::Error::other("no tap"));
        };
        if !self.owned {
            vhost::set_owner(&b.vhost)?;
            vhost::set_mem_table(&b.vhost, &table(memory)?)?;
            self.owned = true;
        }
        let offered = vhost::features(&b.vhost)?;
        vhost::set_features(&b.vhost, features & offered & VHOST_TAKES)?;
        vhost::set_offload(&b.tap, offloads(features))?;
        // From here a failure stops what was started.
        self.running = true;
        let started = (|| -> io::Result<()> {
            for (index, q) in (0u32..).zip(queues) {
                vhost::set_vring_num(&b.vhost, index, u32::from(q.queue.size()))?;
                vhost::set_vring_base(&b.vhost, index, u32::from(q.queue.next_avail()))?;
                vhost::set_vring_addr(
                    &b.vhost,
                    VringAddr {
                        index,
                        flags: 0,
                        desc: host(memory, q.queue.desc_table())?,
                        used: host(memory, q.queue.used_ring())?,
                        avail: host(memory, q.queue.avail_ring())?,
                        log: 0,
                    },
                )?;
                vhost::set_vring_kick(&b.vhost, index, Some(&q.notify))?;
                // vhost-net keeps its own reference to the eventfd.
                let call = interrupt.vector_fd(q.vector);
                vhost::set_vring_call(&b.vhost, index, call.as_ref())?;
            }
            for index in 0..2 {
                vhost::set_backend(&b.vhost, index, Some(&b.tap))?;
            }
            Ok(())
        })();
        if started.is_err() {
            for index in 0..2 {
                let _ = vhost::set_backend(&b.vhost, index, None::<&File>);
            }
            self.running = false;
        }
        started
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tap_gives_the_guest_only_the_offloads_it_took() {
        assert_eq!(offloads(0), 0);
        assert_eq!(offloads(F_GUEST_TSO4), 0);
        assert_eq!(offloads(F_GUEST_CSUM), TUN_F_CSUM);
        assert_eq!(
            offloads(F_GUEST_CSUM | F_GUEST_TSO4 | F_GUEST_TSO6 | F_GUEST_ECN),
            0x0F
        );
        assert_eq!(offloads(F_GUEST_CSUM | F_GUEST_ECN), TUN_F_CSUM);
    }

    #[test]
    fn the_guests_memory_is_one_region_each_at_the_vmms_address() {
        let memory = crate::Memory::from_ranges(&[
            (GuestAddress(0), 0x10_0000),
            (GuestAddress(0x1_0000_0000), 0x20_0000),
        ])
        .unwrap();
        let t = table(&memory).unwrap();
        assert_eq!(t.count, 2);
        assert_eq!(t.regions[1].guest, 0x1_0000_0000);
        assert_eq!(t.regions[1].size, 0x20_0000);
        assert_eq!(t.regions[1].user, host(&memory, 0x1_0000_0000).unwrap());
        assert_ne!(t.regions[0].user, 0);
    }
}
