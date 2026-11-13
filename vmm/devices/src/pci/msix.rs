// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A function's MSI-X table. Each vector has an interrupt number of
//! its own and an eventfd the hypervisor raises it from; the guest's
//! table entry says where the message goes. A vector that is signalled
//! while it is masked stays pending and is delivered when unmasked.

use std::io;
use std::sync::Arc;

use vmm_sys_util::eventfd::EventFd;

use crate::platform::{MsiRoute, Platform};

/// Bytes of a table entry.
const ENTRY_BYTES: usize = 16;
/// The entry's control word: masked.
const ENTRY_MASKED: u8 = 1;
/// The most vectors a table here has.
pub const MAX_VECTORS: usize = 64;

/// The message control's enable bit.
pub const CONTROL_ENABLE: u16 = 1 << 15;
/// The message control's function mask.
pub const CONTROL_FUNCTION_MASK: u16 = 1 << 14;

struct Vector {
    entry: [u8; ENTRY_BYTES],
    gsi: u32,
    fd: EventFd,
    pending: bool,
    routed: Option<MsiRoute>,
}

impl Vector {
    fn masked(&self) -> bool {
        self.entry[12] & ENTRY_MASKED != 0
    }

    fn route(&self) -> MsiRoute {
        let e = &self.entry;
        MsiRoute {
            address: u64::from_le_bytes([e[0], e[1], e[2], e[3], e[4], e[5], e[6], e[7]]),
            data: u32::from_le_bytes([e[8], e[9], e[10], e[11]]),
        }
    }
}

/// The table and its pending bits.
pub struct Msix {
    platform: Arc<dyn Platform>,
    vectors: Vec<Vector>,
    enabled: bool,
    function_masked: bool,
}

impl Msix {
    /// A table of `count` vectors, all masked, as a reset leaves them.
    pub fn new(platform: Arc<dyn Platform>, count: usize) -> io::Result<Self> {
        if count == 0 || count > MAX_VECTORS {
            return Err(io::Error::other("MSI-X vector count"));
        }
        let mut vectors = Vec::with_capacity(count);
        for _ in 0..count {
            let gsi = platform
                .allocate_gsi()
                .ok_or_else(|| io::Error::other("no interrupt number left"))?;
            let fd = EventFd::new(0)?;
            platform.irqfd(gsi, &fd)?;
            let mut entry = [0u8; ENTRY_BYTES];
            entry[12] = ENTRY_MASKED;
            vectors.push(Vector {
                entry,
                gsi,
                fd,
                pending: false,
                routed: None,
            });
        }
        Ok(Self {
            platform,
            vectors,
            enabled: false,
            function_masked: false,
        })
    }

    /// How many vectors the table has.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Never: a table has a vector.
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Bytes of the table.
    pub fn table_bytes(&self) -> usize {
        self.vectors.len() * ENTRY_BYTES
    }

    fn deliverable(&self, index: usize) -> bool {
        self.enabled && !self.function_masked && !self.vectors[index].masked()
    }

    /// Bring vector `index` in line with its entry: its route, and a
    /// pending interrupt when it may now be delivered.
    fn update(&mut self, index: usize) {
        if !self.deliverable(index) {
            return;
        }
        let v = &mut self.vectors[index];
        let route = v.route();
        if v.routed != Some(route) && self.platform.route_msi(v.gsi, Some(route)).is_ok() {
            v.routed = Some(route);
        }
        if v.pending {
            v.pending = false;
            let _ = v.fd.write(1);
        }
    }

    /// The guest wrote the message control word.
    pub fn set_control(&mut self, control: u16) {
        self.enabled = control & CONTROL_ENABLE != 0;
        self.function_masked = control & CONTROL_FUNCTION_MASK != 0;
        for index in 0..self.vectors.len() {
            self.update(index);
        }
    }

    /// Read the table at `offset`; zeros beyond it.
    pub fn read_table(&self, offset: u64, data: &mut [u8]) {
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = usize::try_from(offset)
                .ok()
                .and_then(|o| o.checked_add(i))
                .and_then(|at| Some(self.vectors.get(at / ENTRY_BYTES)?.entry[at % ENTRY_BYTES]))
                .unwrap_or(0);
        }
    }

    /// Write the table at `offset`; nothing beyond it.
    pub fn write_table(&mut self, offset: u64, data: &[u8]) {
        let Ok(offset) = usize::try_from(offset) else {
            return;
        };
        let mut touched = None;
        for (i, value) in data.iter().enumerate() {
            let Some(at) = offset.checked_add(i) else {
                break;
            };
            let Some(vector) = self.vectors.get_mut(at / ENTRY_BYTES) else {
                break;
            };
            // Of the control word only the mask bit is kept.
            vector.entry[at % ENTRY_BYTES] = match at % ENTRY_BYTES {
                12 => value & ENTRY_MASKED,
                13..=15 => 0,
                _ => *value,
            };
            touched = Some(at / ENTRY_BYTES);
        }
        if let Some(index) = touched {
            self.update(index);
        }
    }

    /// Read the pending bits at `offset`.
    pub fn read_pending(&self, offset: u64, data: &mut [u8]) {
        for (i, byte) in data.iter_mut().enumerate() {
            let base = usize::try_from(offset)
                .ok()
                .and_then(|o| o.checked_add(i))
                .and_then(|at| at.checked_mul(8));
            *byte = 0;
            if let Some(base) = base {
                for bit in 0..8 {
                    if self.vectors.get(base + bit).is_some_and(|v| v.pending) {
                        *byte |= 1 << bit;
                    }
                }
            }
        }
    }

    /// Raise vector `index`, or leave it pending while it is masked.
    /// A vector the table does not have is the guest's "none".
    pub fn signal(&mut self, index: u16) {
        let index = usize::from(index);
        if index >= self.vectors.len() {
            return;
        }
        if self.deliverable(index) {
            self.update(index);
            let _ = self.vectors[index].fd.write(1);
        } else {
            self.vectors[index].pending = true;
        }
    }

    /// A reset: every vector masked, nothing pending, the routes gone.
    pub fn reset(&mut self) {
        self.enabled = false;
        self.function_masked = false;
        for v in &mut self.vectors {
            v.entry = [0; ENTRY_BYTES];
            v.entry[12] = ENTRY_MASKED;
            v.pending = false;
            if v.routed.take().is_some() {
                let _ = self.platform.route_msi(v.gsi, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::fake::Recorder;

    fn table() -> (Msix, Arc<Recorder>) {
        let platform = Arc::new(Recorder::default());
        (Msix::new(platform.clone(), 3).unwrap(), platform)
    }

    fn program(msix: &mut Msix, index: u64, address: u64, data: u32) {
        msix.write_table(index * 16, &address.to_le_bytes());
        msix.write_table(index * 16 + 8, &data.to_le_bytes());
        msix.write_table(index * 16 + 12, &0u32.to_le_bytes());
    }

    fn fired(msix: &Msix, index: usize) -> bool {
        // The eventfd counts; a read takes the count.
        msix.vectors[index].fd.write(1).unwrap();
        msix.vectors[index].fd.read().unwrap() > 1
    }

    #[test]
    fn an_unmasked_vector_is_routed_and_raised() {
        let (mut msix, platform) = table();
        msix.set_control(CONTROL_ENABLE);
        program(&mut msix, 1, 0xFEE0_0000, 0x4041);
        let routes = platform.routes.lock().unwrap().clone();
        let route = MsiRoute {
            address: 0xFEE0_0000,
            data: 0x4041,
        };
        assert_eq!(routes, [(25, Some(route))]);
        msix.signal(1);
        assert!(fired(&msix, 1));
        assert!(!fired(&msix, 0));
    }

    #[test]
    fn a_masked_vector_stays_pending_until_it_is_unmasked() {
        let (mut msix, _) = table();
        program(&mut msix, 0, 0xFEE0_0000, 0x21);
        // MSI-X is not enabled yet.
        msix.signal(0);
        assert!(!fired(&msix, 0));
        let mut pending = [0u8; 2];
        msix.read_pending(0, &mut pending);
        assert_eq!(pending, [1, 0]);
        msix.set_control(CONTROL_ENABLE | CONTROL_FUNCTION_MASK);
        assert!(!fired(&msix, 0));
        msix.set_control(CONTROL_ENABLE);
        assert!(fired(&msix, 0));
        msix.read_pending(0, &mut pending);
        assert_eq!(pending, [0, 0]);
    }

    #[test]
    fn accesses_outside_the_table_do_nothing() {
        let (mut msix, platform) = table();
        msix.set_control(CONTROL_ENABLE);
        msix.write_table(3 * 16, &[0xFF; 16]);
        msix.write_table(u64::MAX, &[0xFF; 16]);
        msix.signal(3);
        msix.signal(0xFFFF);
        let mut data = [0xAAu8; 8];
        msix.read_table(3 * 16 - 4, &mut data);
        assert_eq!(data, [1, 0, 0, 0, 0, 0, 0, 0]);
        msix.read_pending(u64::MAX, &mut data);
        assert_eq!(data, [0; 8]);
        assert!(platform.routes.lock().unwrap().is_empty());
        assert!(Msix::new(platform.clone(), 0).is_err());
        assert!(Msix::new(platform, MAX_VECTORS + 1).is_err());
    }

    #[test]
    fn a_reset_masks_every_vector_and_drops_the_routes() {
        let (mut msix, platform) = table();
        msix.set_control(CONTROL_ENABLE);
        program(&mut msix, 2, 0xFEE0_1000, 0x30);
        msix.reset();
        assert_eq!(platform.routes.lock().unwrap().last(), Some(&(26, None)));
        msix.signal(2);
        assert!(!fired(&msix, 2));
    }
}
