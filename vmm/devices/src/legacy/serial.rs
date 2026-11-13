// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The 16550 at the first serial port's address: `vm-superio`'s, with
//! its interrupt on an eventfd and its output to whatever the VMM
//! gives it.

use std::io::{self, Write};

use vm_superio::serial::NoEvents;
use vm_superio::{Serial as Uart, Trigger};
use vmm_sys_util::eventfd::EventFd;

use crate::bus::BusDevice;

/// The interrupt line: written when the port wants attention.
pub struct IrqLine(EventFd);

impl Trigger for IrqLine {
    type E = io::Error;

    fn trigger(&self) -> io::Result<()> {
        self.0.write(1)
    }
}

/// Where the port's output goes.
pub type Output = Box<dyn Write + Send>;

/// The serial port.
pub struct Serial {
    uart: Uart<IrqLine, NoEvents, Output>,
}

impl Serial {
    /// A port whose interrupt is `irq` and whose output goes to `out`.
    pub fn new(irq: EventFd, out: Output) -> Self {
        Self {
            uart: Uart::new(IrqLine(irq), out),
        }
    }

    /// Bytes the receive buffer has room for.
    pub fn room(&self) -> usize {
        self.uart.fifo_capacity()
    }

    /// Give the guest input. How many bytes were taken: no more than
    /// there was [`Serial::room`] for.
    pub fn receive(&mut self, bytes: &[u8]) -> usize {
        let take = bytes.len().min(self.room());
        if take == 0 {
            return 0;
        }
        self.uart.enqueue_raw_bytes(&bytes[..take]).unwrap_or(0)
    }
}

impl BusDevice for Serial {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        data.fill(0xFF);
        if let (Ok(offset), Some(first)) = (u8::try_from(offset), data.first_mut()) {
            *first = self.uart.read(offset);
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        if let (Ok(offset), Some(first)) = (u8::try_from(offset), data.first()) {
            // A failed interrupt or a failed write of the output is the
            // host's trouble, not the guest's.
            let _ = self.uart.write(offset, *first);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn port() -> (Serial, Arc<Mutex<Vec<u8>>>) {
        let out = Arc::new(Mutex::new(Vec::new()));
        let irq = EventFd::new(0).unwrap();
        (Serial::new(irq, Box::new(Sink(out.clone()))), out)
    }

    #[test]
    fn what_the_guest_transmits_reaches_the_output() {
        let (mut serial, out) = port();
        for byte in b"REF" {
            serial.write(0, &[*byte]);
        }
        assert_eq!(*out.lock().unwrap(), b"REF");
        // The line status says the transmitter is empty.
        let mut lsr = [0u8];
        serial.read(5, &mut lsr);
        assert_eq!(lsr[0] & 0x60, 0x60);
    }

    #[test]
    fn input_is_read_back_and_bounded_by_the_buffer() {
        let (mut serial, _) = port();
        let room = serial.room();
        assert!(room > 0);
        let flood = vec![b'x'; room + 100];
        assert_eq!(serial.receive(&flood), room);
        assert_eq!(serial.receive(b"y"), 0);
        let mut byte = [0u8];
        serial.read(0, &mut byte);
        assert_eq!(byte, [b'x']);
        assert_eq!(serial.receive(b"yz"), 1);
    }

    #[test]
    fn odd_accesses_do_not_panic() {
        let (mut serial, _) = port();
        serial.write(200, &[1]);
        serial.write(1, &[]);
        let mut wide = [0u8; 4];
        serial.read(u64::MAX, &mut wide);
        serial.read(7, &mut wide);
        assert_eq!(wide[1..], [0xFF; 3]);
    }
}
