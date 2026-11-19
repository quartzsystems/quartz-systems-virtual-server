// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Capture of QEMU's standard output (the guest's serial port) and
//! standard error, as chunks on one channel.

use std::io::Read;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

/// Forward chunks from both pipes into one channel. The channel closes
/// when both pipes reach their end, which is how QEMU's exit is noticed.
pub fn stream<O, E>(stdout: Option<O>, stderr: Option<E>) -> Receiver<String>
where
    O: Read + Send + 'static,
    E: Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    if let Some(out) = stdout {
        pump(out, tx.clone());
    }
    if let Some(err) = stderr {
        pump(err, tx);
    }
    rx
}

fn pump<R: Read + Send + 'static>(mut reader: R, tx: Sender<String>) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).replace('\r', "");
                    if tx.send(text).is_err() {
                        break;
                    }
                }
            }
        }
    });
}
