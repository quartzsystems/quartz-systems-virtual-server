// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Where PID 1 prints: `/dev/console` (the local display) and
//! `/dev/ttyS0` (the serial port). A device that will not open is
//! skipped; nothing here can fail the IPL.

use std::fs::{File, OpenOptions};
use std::io::Write;

/// The local display, as the kernel names its console.
const DISPLAY: &str = "/dev/console";
/// The serial port.
const SERIAL: &str = "/dev/ttyS0";

/// The two devices, each open or absent.
pub struct Console {
    display: Option<File>,
    serial: Option<File>,
}

fn open(path: &str) -> Option<File> {
    OpenOptions::new().write(true).open(path).ok()
}

fn put(dev: &mut Option<File>, line: &str) {
    if let Some(f) = dev {
        let _ = f.write_all(format!("{line}\n").as_bytes());
        let _ = f.flush();
    }
}

impl Console {
    /// Open both devices. Call once `/dev` is mounted.
    pub fn open() -> Self {
        Self {
            display: open(DISPLAY),
            serial: open(SERIAL),
        }
    }

    /// One line on the display and the serial port.
    pub fn line(&mut self, line: &str) {
        put(&mut self.display, line);
        put(&mut self.serial, line);
    }

    /// One line on the serial port alone.
    pub fn serial(&mut self, line: &str) {
        put(&mut self.serial, line);
    }
}
