// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The console socket: `<run>/partitions/<name>/console.sock`. A
//! client reads what the guest transmits from the moment it connects
//! and writes what the guest is to receive. The socket is bound while
//! the process is root and is root's alone.

use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use devices::legacy::serial::Serial;

use super::{Clients, patient};

/// The socket's file name.
pub const FILE: &str = "console.sock";
/// The most clients at once.
const MAX_CLIENTS: usize = 4;
/// How long the guest is given to take a byte before it is dropped.
const INPUT_PATIENCE: Duration = Duration::from_secs(5);

/// Bind the socket in `dir`, making the directory.
pub fn bind(dir: &Path) -> io::Result<UnixListener> {
    fs::create_dir_all(dir)?;
    let path = dir.join(FILE);
    // One left by a process that was killed.
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Give the guest `bytes`, waiting while its receive buffer is full.
fn feed(serial: &Arc<Mutex<Serial>>, mut bytes: &[u8]) {
    let mut waited = Duration::ZERO;
    let pause = Duration::from_millis(2);
    while !bytes.is_empty() && waited < INPUT_PATIENCE {
        let taken = serial.lock().map_or(0, |mut s| s.receive(bytes));
        bytes = &bytes[taken..];
        if taken == 0 {
            thread::sleep(pause);
            waited += pause;
        } else {
            waited = Duration::ZERO;
        }
    }
}

fn pump(mut client: UnixStream, serial: &Arc<Mutex<Serial>>) {
    let mut buffer = [0u8; 256];
    loop {
        match client.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => feed(serial, &buffer[..n]),
        }
    }
}

/// Start the thread that takes connections.
pub fn spawn(
    listener: UnixListener,
    serial: Arc<Mutex<Serial>>,
    clients: Clients,
) -> io::Result<()> {
    thread::Builder::new()
        .name("console".into())
        .spawn(move || {
            for client in listener.incoming().flatten() {
                let Ok(output) = client.try_clone() else {
                    continue;
                };
                patient(&output);
                let admitted = clients.lock().is_ok_and(|mut clients| {
                    let room = clients.len() < MAX_CLIENTS;
                    if room {
                        clients.push(output);
                    }
                    room
                });
                if !admitted {
                    continue;
                }
                let serial = serial.clone();
                let _ = thread::Builder::new()
                    .name("console-in".into())
                    .spawn(move || pump(client, &serial));
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use devices::bus::BusDevice;
    use std::io::Write;
    use vmm_sys_util::eventfd::EventFd;

    #[test]
    fn what_a_client_writes_is_the_guest_s_input() {
        let dir = std::env::temp_dir().join(format!("qsvmm-console-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let listener = bind(&dir).unwrap();
        let mode = fs::metadata(dir.join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // Binding again takes the place of a socket left behind.
        drop(listener);
        let listener = bind(&dir).unwrap();

        let serial = Arc::new(Mutex::new(Serial::new(
            EventFd::new(0).unwrap(),
            Box::new(io::sink()),
        )));
        let clients: Clients = Arc::default();
        spawn(listener, serial.clone(), clients.clone()).unwrap();
        let mut client = UnixStream::connect(dir.join(FILE)).unwrap();
        client.write_all(b"qshwqual\n").unwrap();

        let mut got = Vec::new();
        for _ in 0..2000 {
            let mut lsr = [0u8];
            let mut s = serial.lock().unwrap();
            s.read(5, &mut lsr);
            if lsr[0] & 1 != 0 {
                let mut byte = [0u8];
                s.read(0, &mut byte);
                got.push(byte[0]);
            } else {
                drop(s);
                if got.len() == 9 {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(got, b"qshwqual\n");
        assert_eq!(clients.lock().unwrap().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
