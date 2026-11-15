// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A console's viewer on the socket: after the answer the connection
//! stays open, [`ConsoleOut`] going to the client and [`ConsoleIn`]
//! coming from it, until either side closes it or another viewer takes
//! the console over. With the console mode serial, the serial port's
//! bytes are relayed from the VMM's console socket instead.

use std::io::{self, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::PoisonError;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use ipc::frame::{read_frame_max, write_frame};

use super::watch::Shared;
use crate::manager::VIEWER_QUEUE;
use crate::proto::{ConsoleIn, ConsoleOut, MAX_FRAME, Refused};

fn lock(
    shared: &Shared,
) -> std::sync::MutexGuard<'_, crate::manager::Manager<super::machine::Machine>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Serve a viewer of `id`'s console on `stream`.
pub fn serve(
    shared: &Shared,
    serial_socket: &Path,
    mut stream: UnixStream,
    id: u32,
    take_over: bool,
) -> Result<(), ipc::WireError> {
    let (to, from) = mpsc::sync_channel(VIEWER_QUEUE);
    let attached = lock(shared).console_attach(id, take_over, to);
    let (viewer, serial) = match attached {
        Ok(v) => v,
        Err(refused) => {
            return write_frame(&mut stream, &ipc::encode(&Err::<bool, Refused>(refused)));
        }
    };
    write_frame(&mut stream, &ipc::encode(&Ok::<bool, Refused>(serial)))?;
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let result = if serial {
        relay_serial(shared, serial_socket, &stream, &from, id, viewer)
    } else {
        frames(shared, &stream, from, id, viewer)
    };
    lock(shared).console_detach(id, viewer);
    let _ = stream.shutdown(Shutdown::Both);
    result
}

/// The console port's frames out, the client's keys in.
fn frames(
    shared: &Shared,
    stream: &UnixStream,
    from: Receiver<ConsoleOut>,
    id: u32,
    viewer: u64,
) -> Result<(), ipc::WireError> {
    let reader = {
        let (shared, mut input) = (shared.clone(), stream.try_clone()?);
        thread::spawn(move || {
            while let Ok(frame) = read_frame_max(&mut input, MAX_FRAME) {
                let Ok(message) = ipc::decode::<ConsoleIn>(&frame) else {
                    break;
                };
                if !lock(&shared).console_input(id, viewer, &message) {
                    break;
                }
            }
            // The client is gone: the viewer's channel closes with it.
            lock(&shared).console_detach(id, viewer);
        })
    };
    let mut out = stream.try_clone()?;
    for message in from {
        let end = matches!(message, ConsoleOut::Ended { .. });
        if write_frame(&mut out, &ipc::encode(&message)).is_err() || end {
            break;
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    let _ = reader.join();
    Ok(())
}

/// The serial port's bytes both ways, from the VMM's console socket.
fn relay_serial(
    shared: &Shared,
    socket: &Path,
    stream: &UnixStream,
    from: &Receiver<ConsoleOut>,
    id: u32,
    viewer: u64,
) -> Result<(), ipc::WireError> {
    let serial = UnixStream::connect(socket).map_err(ipc::WireError::from)?;
    serial.set_read_timeout(Some(Duration::from_millis(200)))?;
    let writer = {
        let (shared, mut input, mut port) =
            (shared.clone(), stream.try_clone()?, serial.try_clone()?);
        thread::spawn(move || {
            while let Ok(frame) = read_frame_max(&mut input, MAX_FRAME) {
                match ipc::decode::<ConsoleIn>(&frame) {
                    Ok(ConsoleIn::Bytes { bytes }) => {
                        if !lock(&shared).console_viewer(id, viewer)
                            || port.write_all(&bytes).is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            let _ = port.shutdown(Shutdown::Both);
        })
    };
    let mut out = stream.try_clone()?;
    let mut buf = vec![0u8; 4096];
    let mut port = &serial;
    loop {
        match from.try_recv() {
            Ok(message @ ConsoleOut::Ended { .. }) => {
                let _ = write_frame(&mut out, &ipc::encode(&message));
                break;
            }
            Ok(_) | Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }
        match port.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let message = ConsoleOut::Serial {
                    bytes: buf[..n].to_vec(),
                };
                if write_frame(&mut out, &ipc::encode(&message)).is_err() {
                    break;
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    let _ = serial.shutdown(Shutdown::Both);
    let _ = stream.shutdown(Shutdown::Both);
    let _ = writer.join();
    Ok(())
}

/// Send a dump's bytes from `file` on `stream`, a frame each, then an
/// empty frame.
pub fn send_file(stream: &mut UnixStream, file: &mut std::fs::File) -> io::Result<()> {
    let mut buf = vec![0u8; crate::proto::DUMP_PIECE];
    loop {
        let n = file.read(&mut buf)?;
        write_frame(stream, &buf[..n]).map_err(io::Error::other)?;
        if n == 0 {
            return stream.flush();
        }
    }
}
