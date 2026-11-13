// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvshook partition`: the service that lets the harness see what
//! partitions say. For every partition the partition manager has
//! activated it relays the partition's serial output to the host's
//! serial port, each line after the partition's name, and the lines of
//! its VMM's report that are no reference code. The codes themselves
//! the partition manager posts, as it does in a release image.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use partd::{Calls as _, Client};

use super::{relayed, reported};

/// The host's serial port.
const HOST_SERIAL: &str = "/dev/ttyS0";
/// How often the partitions are looked at.
const PAUSE: Duration = Duration::from_millis(100);

/// The partition manager's socket.
pub fn client() -> Client {
    Client::new(partd::proto::socket(release::RUN_DIR))
}

/// The console socket of the partition `id`.
pub fn console_socket(id: u32) -> PathBuf {
    qsvmm::socket_dir(Path::new(release::RUN_DIR), id).join(qsvmm::CONSOLE_SOCKET)
}

fn serial_line(line: &str) {
    if let Ok(mut port) = OpenOptions::new().write(true).open(HOST_SERIAL) {
        let _ = port.write_all(format!("{line}\r\n").as_bytes());
    }
}

/// Relay the partition's serial output, a line at a time, for as long
/// as the console socket is there. `over` says the VMM has ended: a
/// socket not yet there by then will not be this one's.
fn follow_serial(id: u32, name: String, over: Arc<AtomicBool>) {
    thread::spawn(move || {
        let socket = console_socket(id);
        // The socket is there a moment after `qsvmm` starts; on a busy
        // host that moment may be long.
        let console = loop {
            if over.load(Ordering::Relaxed) {
                return;
            }
            match UnixStream::connect(&socket) {
                Ok(stream) => break stream,
                Err(_) => thread::sleep(Duration::from_millis(25)),
            }
        };
        let mut line = Vec::new();
        let mut reader = BufReader::new(console);
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    serial_line(&relayed(&name, text.trim_end_matches(['\r', '\n'])));
                }
            }
        }
    });
}

/// Relay the lines of the VMM's report that are no reference code,
/// until the VMM has ended and what it wrote is read.
fn follow_report(id: u32, name: String, over: Arc<AtomicBool>) {
    thread::spawn(move || {
        let path = partd::proto::report(Path::new(release::RUN_DIR), id);
        let mut at = 0u64;
        let mut partial = String::new();
        loop {
            let ended = over.load(Ordering::Relaxed);
            if let Ok(mut file) = File::open(&path)
                && file.seek(SeekFrom::Start(at)).is_ok()
            {
                let mut text = String::new();
                if let Ok(read) = file.take(1 << 20).read_to_string(&mut text) {
                    at += read as u64;
                    partial.push_str(&text);
                }
            }
            while let Some(end) = partial.find('\n') {
                let line: String = partial.drain(..=end).collect();
                let line = line.trim_end();
                let is_code = line.starts_with(refcode::WORD);
                let is_end = line.starts_with(qsvmm::END_WORD);
                if !is_code && !is_end && !line.is_empty() {
                    serial_line(&reported(&name, line));
                }
            }
            if ended {
                return;
            }
            thread::sleep(PAUSE);
        }
    });
}

/// Run the service. It says it is ready at once: the host's base does
/// not wait for a partition.
pub fn run() -> io::Result<Infallible> {
    supervise::ready::signal()?;
    let mut client = client();
    // The VMMs being relayed, by partition and process, each with the
    // flag that says it has ended.
    let relays: Mutex<Vec<(u32, u32, Arc<AtomicBool>)>> = Mutex::new(Vec::new());
    loop {
        let running: BTreeSet<(u32, u32, String)> = client
            .list()
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.pid != 0)
            .map(|p| (p.id, p.pid, p.name))
            .collect();
        let mut relays = relays.lock().unwrap_or_else(PoisonError::into_inner);
        relays.retain(|(id, pid, over)| {
            let there = running.iter().any(|(i, p, _)| (i, p) == (id, pid));
            if !there {
                over.store(true, Ordering::Relaxed);
            }
            there
        });
        for (id, pid, name) in running {
            if relays.iter().any(|(i, p, _)| (*i, *p) == (id, pid)) {
                continue;
            }
            let over = Arc::new(AtomicBool::new(false));
            follow_serial(id, name.clone(), over.clone());
            follow_report(id, name, over.clone());
            relays.push((id, pid, over));
        }
        drop(relays);
        thread::sleep(PAUSE);
    }
}
