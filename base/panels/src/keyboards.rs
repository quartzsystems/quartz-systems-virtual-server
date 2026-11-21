// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The display's keyboards, as they come and go. `console`'s readers
//! open the keyboards there are when they start; a USB keyboard (a
//! server's own, or its management controller's virtual one) is there
//! only once its controller has been probed, seconds after the console
//! starts, and may be plugged in later still. So the input devices are
//! looked at every second, and each one that reports keys is read from
//! the moment it is there until it is gone. A mouse or a tablet reports
//! keys too (its buttons), so it is read the same way. `console` decodes
//! each event, keys and the pointer alike, with the device's axes.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use console::Command;
use console::keyboard::{abs_ranges, decode};
use console::pointer::EVENT_LEN;

/// How often the devices are looked at.
const EVERY: Duration = Duration::from_secs(1);

/// Read every keyboard there is now and every one that comes later.
pub fn watch(tx: &Sender<Command>) {
    let tx = tx.clone();
    let open: Arc<Mutex<HashSet<String>>> = Arc::default();
    thread::spawn(move || {
        loop {
            for name in keyboards() {
                let mut reading = open.lock().unwrap_or_else(PoisonError::into_inner);
                if reading.contains(&name) {
                    continue;
                }
                let Ok(file) = File::open(Path::new("/dev/input").join(&name)) else {
                    continue;
                };
                reading.insert(name.clone());
                let (tx, open) = (tx.clone(), Arc::clone(&open));
                thread::spawn(move || {
                    pump(file, &tx);
                    // Gone: read again when it comes back.
                    open.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&name);
                });
            }
            thread::sleep(EVERY);
        }
    });
}

/// The event devices that report keys.
fn keyboards() -> Vec<String> {
    let Ok(dir) = fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };
    let mut names: Vec<String> = dir
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("event"))
        .filter(|n| reports_keys(&Path::new("/sys/class/input").join(n)))
        .collect();
    names.sort();
    names
}

/// `capabilities/ev` has bit 1 (`EV_KEY`) set.
fn reports_keys(sys: &Path) -> bool {
    let ev = fs::read_to_string(sys.join("device/capabilities/ev")).unwrap_or_default();
    u64::from_str_radix(ev.trim(), 16).is_ok_and(|bits| bits & 0b10 != 0)
}

/// Pass the device's key and pointer events on until it is gone.
fn pump(mut file: File, tx: &Sender<Command>) {
    let ranges = abs_ranges(&file);
    let mut buf = [0u8; EVENT_LEN];
    while file.read_exact(&mut buf).is_ok() {
        if let Some(command) = decode(&buf, &ranges)
            && tx.send(command).is_err()
        {
            return;
        }
    }
}
