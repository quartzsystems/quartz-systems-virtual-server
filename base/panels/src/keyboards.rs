// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The display's keyboards, as they come and go. `console`'s readers
//! open the keyboards there are when they start; a USB keyboard (a
//! server's own, or its management controller's virtual one) is there
//! only once its controller has been probed, seconds after the console
//! starts, and may be plugged in later still. So the input devices are
//! looked at every second, and each one that reports keys is read from
//! the moment it is there until it is gone. The events go to `console`
//! as they are, which maps them.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use console::Command;
use console::keyboard::KeyEvent;

/// `struct input_event` on `x86_64`: timeval (16), type (2), code (2),
/// value (4).
const EVENT_LEN: usize = 24;
/// The event type of a key.
const EV_KEY: u16 = 1;
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

/// Pass the device's key events on until it is gone.
fn pump(mut file: File, tx: &Sender<Command>) {
    let mut buf = [0u8; EVENT_LEN];
    while file.read_exact(&mut buf).is_ok() {
        if let Some(ev) = key(&buf)
            && tx.send(Command::Key(ev)).is_err()
        {
            return;
        }
    }
}

/// The key of one `input_event`, if it is one.
fn key(buf: &[u8; EVENT_LEN]) -> Option<KeyEvent> {
    if u16::from_ne_bytes([buf[16], buf[17]]) != EV_KEY {
        return None;
    }
    Some(KeyEvent {
        code: u16::from_ne_bytes([buf[18], buf[19]]),
        pressed: i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]) != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT_LEN] {
        let mut b = [0u8; EVENT_LEN];
        b[16..18].copy_from_slice(&kind.to_ne_bytes());
        b[18..20].copy_from_slice(&code.to_ne_bytes());
        b[20..24].copy_from_slice(&value.to_ne_bytes());
        b
    }

    #[test]
    fn a_key_is_passed_on_and_anything_else_is_not() {
        let down = key(&event(EV_KEY, 28, 1)).unwrap();
        assert_eq!((down.code, down.pressed), (28, true));
        let up = key(&event(EV_KEY, 28, 0)).unwrap();
        assert!(!up.pressed);
        // A repeat is a press.
        assert!(key(&event(EV_KEY, 28, 2)).unwrap().pressed);
        // A sync or a relative move is none.
        assert!(key(&event(0, 0, 0)).is_none());
        assert!(key(&event(2, 0, 5)).is_none());
    }
}
