// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The keyboard controller's reset line: `vm-superio`'s i8042, of
//! which only the pulse on the reset line matters. The FADT names its
//! command port as the ACPI reset register, and a kernel that does not
//! ask ACPI pulses it by the keyboard controller's own rule. There is
//! no keyboard behind it.

use std::sync::Arc;

use vm_superio::{I8042Device, Trigger};

use crate::bus::BusDevice;
use crate::platform::{Request, Requests};

/// The reset line: pulled, the partition ends with a restart asked for.
pub struct ResetLine(Arc<Requests>);

impl Trigger for ResetLine {
    type E = std::convert::Infallible;

    fn trigger(&self) -> Result<(), Self::E> {
        (self.0)(Request::Restart);
        Ok(())
    }
}

/// The ports 0x60 to 0x64.
pub struct Reset {
    i8042: I8042Device<ResetLine>,
}

impl Reset {
    /// The reset line, telling `requests` when it is pulsed.
    pub fn new(requests: Arc<Requests>) -> Self {
        Self {
            i8042: I8042Device::new(ResetLine(requests)),
        }
    }
}

impl BusDevice for Reset {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        data.fill(0xFF);
        if let (Ok(offset), Some(first)) = (u8::try_from(offset), data.first_mut()) {
            *first = self.i8042.read(offset);
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        if let (Ok(offset), Some(first)) = (u8::try_from(offset), data.first()) {
            let _ = self.i8042.write(offset, *first);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn the_reset_pulse_and_nothing_else_asks_for_a_restart() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let requests: Arc<Requests> = Arc::new(Box::new(move |r| log.lock().unwrap().push(r)));
        let mut reset = Reset::new(requests);
        // Offset 4 is port 0x64, the command port.
        reset.write(4, &[0xAA]);
        reset.write(0, &[0xFE]);
        reset.write(9, &[0xFE]);
        let mut status = [0u8];
        reset.read(4, &mut status);
        assert!(seen.lock().unwrap().is_empty());
        reset.write(4, &[0xFE]);
        assert_eq!(*seen.lock().unwrap(), [Request::Restart]);
    }
}
