// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The ports of the partition's virtio-console on the host's side
//! (`docs/VMM.md`): the service channel's two. The partition manager
//! makes a listening Unix socket for each and hands it over as a
//! descriptor; `qsvmm` takes one peer at a time on
//! it and carries bytes between the peer and the port. It reads none of
//! them: the protocol is the partition manager's.
//!
//! A peer that goes away is normal: the partition manager restarting.
//! The guest is told the host's side closed, and opened again when the
//! next peer connects.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::thread;

use devices::virtio::console::{Link, Port};

use crate::config::Partition;

/// The most bytes read from a peer at once, and written to it.
const PIECE: usize = 32 * 1024;

/// The two ports, each with its listener.
pub struct Ports {
    /// The device's ports, in order: the service port, the console port.
    pub ports: Vec<Port>,
    listeners: Vec<UnixListener>,
}

/// The ports of `partition`, when it has any: each named as its
/// configuration names it, which the partition manager takes from the
/// protocol's crate.
pub fn open(partition: &Partition) -> io::Result<Option<Ports>> {
    if partition.config.ports.is_empty() {
        return Ok(None);
    }
    let mut ports = Vec::new();
    let mut listeners = Vec::new();
    for port in &partition.config.ports {
        let fd = crate::net::descriptor(&port.socket).ok_or_else(|| {
            io::Error::other(format!("{} is no descriptor", port.socket.display()))
        })?;
        listeners.push(
            crate::sys::inherited_listener(fd)
                .map_err(|e| io::Error::other(format!("port {}: {e}", port.name)))?,
        );
        ports.push(Port {
            name: port.name.clone(),
            link: Arc::new(Link::new()?),
        });
    }
    Ok(Some(Ports { ports, listeners }))
}

/// Carry one peer's bytes until it goes.
fn serve(link: &Arc<Link>, stream: &UnixStream) {
    let n = link.connect();
    let writer = {
        let (link, stream) = (link.clone(), stream.try_clone());
        thread::Builder::new()
            .name("channel-out".into())
            .spawn(move || {
                let Ok(mut stream) = stream else {
                    return;
                };
                while let Some(bytes) = link.from_guest(n, PIECE) {
                    if stream.write_all(&bytes).is_err() {
                        break;
                    }
                }
                let _ = stream.shutdown(Shutdown::Both);
            })
    };
    let mut buf = vec![0u8; PIECE];
    let mut reader = stream;
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(got) => {
                if !link.to_guest(n, &buf[..got]) {
                    break;
                }
            }
        }
    }
    link.disconnect(n);
    let _ = stream.shutdown(Shutdown::Both);
    if let Ok(writer) = writer {
        let _ = writer.join();
    }
}

/// Serve each port's listener on a thread of its own.
pub fn spawn(ports: Ports) -> io::Result<()> {
    for (port, listener) in ports.ports.into_iter().zip(ports.listeners) {
        thread::Builder::new()
            .name("channel".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    match stream {
                        Ok(stream) => serve(&port.link, &stream),
                        Err(e) => {
                            eprintln!("qsvmm: {}: {e}", port.name);
                            return;
                        }
                    }
                }
            })?;
    }
    Ok(())
}
