// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The ACPI sleep control register of a hardware-reduced machine, and
//! the status register beside it. Writing the sleep type of S5 with
//! the enable bit powers the partition off; no other sleep state
//! exists.

use std::sync::Arc;

use arch::layout::SLEEP_TYPE_S5;

use crate::bus::BusDevice;
use crate::platform::{Request, Requests};

/// `SLP_EN`.
const SLEEP_ENABLE: u8 = 1 << 5;
/// `SLP_TYP` is three bits from bit 2.
const SLEEP_TYPE_SHIFT: u8 = 2;
const SLEEP_TYPE_MASK: u8 = 0x7;

/// The two registers.
pub struct Power {
    requests: Arc<Requests>,
}

impl Power {
    /// The registers, telling `requests` when the guest powers off.
    pub fn new(requests: Arc<Requests>) -> Self {
        Self { requests }
    }
}

impl BusDevice for Power {
    fn read(&mut self, _offset: u64, data: &mut [u8]) {
        // Control reads as written-and-done; status has no wake event.
        data.fill(0);
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        let Some(value) = data.first().copied() else {
            return;
        };
        let sleep_type = (value >> SLEEP_TYPE_SHIFT) & SLEEP_TYPE_MASK;
        if offset == 0 && value & SLEEP_ENABLE != 0 && sleep_type == SLEEP_TYPE_S5 {
            (self.requests)(Request::PowerOff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn s5_with_the_enable_bit_powers_off() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut power = Power::new(Arc::new(Box::new(move |r| log.lock().unwrap().push(r))));
        power.write(0, &[5 << 2]); // the type without the enable bit
        power.write(0, &[(3 << 2) | SLEEP_ENABLE]); // another state
        power.write(1, &[(5 << 2) | SLEEP_ENABLE]); // the status register
        power.write(0, &[]);
        assert!(seen.lock().unwrap().is_empty());
        power.write(0, &[(5 << 2) | SLEEP_ENABLE]);
        assert_eq!(*seen.lock().unwrap(), [Request::PowerOff]);
    }
}
