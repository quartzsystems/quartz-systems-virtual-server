// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A hostile system, for the suite that holds the partition manager to
//! its word that a guest cannot disturb it (`service-hostile`). It acts
//! only in a test image running as a partition, and only when one of
//! the partition's disks begins with [`MARKER`], which a suite gives it:
//! it floods its service port with what is not frames, sends frames
//! that do not read and frames that claim to be too long, a flood of
//! well-formed ones, and opens and closes the port over and over. What
//! it does it says on the partition's serial port.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// What a disk begins with to make this partition hostile.
pub const MARKER: &[u8] = b"QSVS-HOSTILE-GUEST\n";
/// The partition's serial port.
const SERIAL: &str = "/dev/ttyS0";

fn say(line: &str) {
    if let Ok(mut port) = OpenOptions::new().write(true).open(SERIAL) {
        let _ = port.write_all(format!("hostile: {line}\r\n").as_bytes());
    }
}

/// Whether a SCSI disk of the partition begins with the marker.
fn marked() -> bool {
    fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let whole_disk =
                name.starts_with("sd") && name[2..].bytes().all(|b| b.is_ascii_lowercase());
            let mut head = [0u8; 64];
            whole_disk
                && File::open(e.path())
                    .and_then(|mut f| f.read_exact(&mut head))
                    .is_ok()
                && head.starts_with(MARKER)
        })
}

/// The device of the port called `name`, by its name in sysfs.
fn port(name: &str) -> Option<PathBuf> {
    fs::read_dir("/sys/class/virtio-ports")
        .ok()?
        .flatten()
        .find(|e| fs::read_to_string(e.path().join("name")).is_ok_and(|n| n.trim() == name))
        .map(|e| Path::new("/dev").join(e.file_name()))
}

fn open(path: &Path) -> Option<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc_nonblock())
        .open(path)
        .ok()
}

fn libc_nonblock() -> i32 {
    // O_NONBLOCK on x86-64 Linux.
    0o4000
}

/// Write `bytes` as far as the port takes them within `secs`.
fn push(port: &mut File, bytes: &[u8], until: Instant) -> usize {
    let mut done = 0;
    while done < bytes.len() && Instant::now() < until {
        match port.write(&bytes[done..]) {
            Ok(n) => done += n,
            Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
    done
}

fn act() {
    say("begun");
    let deadline = Instant::now() + Duration::from_secs(120);
    let path = loop {
        if let Some(p) = port(qsipart::SERVICE_PORT) {
            break p;
        }
        if Instant::now() > deadline {
            say("no service port");
            return;
        }
        thread::sleep(Duration::from_millis(500));
    };
    let Some(mut p) = open(&path) else {
        say("the service port does not open");
        return;
    };
    let mut x: u32 = 0x9E37_79B9;
    let junk: Vec<u8> = (0..2 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x.to_le_bytes()[1]
        })
        .collect();
    let n = push(&mut p, &junk, Instant::now() + Duration::from_secs(30));
    say(&format!("junk {n} bytes"));
    let bad = qsipart::frame::encode(qsipart::service::kind::STATE, &[9]).unwrap_or_default();
    let n = push(
        &mut p,
        &bad.repeat(200),
        Instant::now() + Duration::from_secs(10),
    );
    say(&format!("frames that do not read, {n} bytes"));
    // A header that claims more than a frame holds, its check right.
    let kind: u16 = qsipart::service::kind::HELLO;
    let len: u16 = u16::MAX;
    let mut long = b"QI".to_vec();
    long.extend_from_slice(&kind.to_le_bytes());
    long.extend_from_slice(&len.to_le_bytes());
    long.extend_from_slice(&(!(kind ^ len)).to_le_bytes());
    let n = push(
        &mut p,
        &long.repeat(1000),
        Instant::now() + Duration::from_secs(10),
    );
    say(&format!("frames too long, {n} bytes"));
    let state = qsipart::service::FromSystem::State(qsipart::service::State {
        state: qsipart::service::SysState::Complete,
        fix_level: "x".repeat(1000),
        addresses: vec!["y".repeat(100); 100],
    })
    .encode()
    .unwrap_or_default();
    let n = push(
        &mut p,
        &state.repeat(5000),
        Instant::now() + Duration::from_secs(30),
    );
    say(&format!("a flood of frames, {n} bytes"));
    drop(p);
    let mut opened = 0;
    for _ in 0..1000 {
        if open(&path).is_some() {
            opened += 1;
        }
    }
    say(&format!("opened and closed {opened} times"));
    say("done");
}

/// Act on a thread when this partition was made hostile.
pub fn spawn_if_marked() {
    thread::spawn(|| {
        // The disks are a moment in coming after the IPL.
        for _ in 0..30 {
            if marked() {
                act();
                return;
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}
