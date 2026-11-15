// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The connections of each partition's service channel. The partition
//! manager makes a listening socket for each port and hands it to the
//! VMM; here it connects to them, one thread a port, and carries what
//! arrives to the manager and what the manager sends to the socket. A
//! connection that goes is made again: the system's side never needs
//! the manager to have been there all along. One the manager closed for
//! nonsense is made again after a while.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use super::watch::Shared;
use crate::manager::Port;

/// The most frames that wait for a port's socket.
const QUEUE: usize = 256;
/// How long a connection closed for nonsense stays closed.
pub const CLOSED_FOR: Duration = Duration::from_secs(30);
/// How often a missing connection is tried again.
const RETRY: Duration = Duration::from_millis(500);
/// The most bytes read at once.
const PIECE: usize = 32 * 1024;

/// A connection the manager can send to and close.
struct Open {
    to: SyncSender<Vec<u8>>,
    stream: UnixStream,
    number: u64,
}

/// Every connection, shared by the machine and the threads that keep
/// them.
#[derive(Clone, Default)]
pub struct Links {
    open: Arc<Mutex<BTreeMap<(u32, Port), Open>>>,
    closed: Arc<Mutex<BTreeMap<(u32, Port), Instant>>>,
    numbers: Arc<Mutex<u64>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Links {
    /// Send `frame` on `id`'s `port`; dropped when there is no
    /// connection or the system does not take what it is sent.
    pub fn send(&self, id: u32, port: Port, frame: Vec<u8>) {
        if let Some(open) = lock(&self.open).get(&(id, port)) {
            let _ = open.to.try_send(frame);
        }
    }

    /// Close `id`'s `port`; it is made again after [`CLOSED_FOR`].
    pub fn close(&self, id: u32, port: Port) {
        if let Some(open) = lock(&self.open).remove(&(id, port)) {
            let _ = open.stream.shutdown(Shutdown::Both);
        }
        lock(&self.closed).insert((id, port), Instant::now() + CLOSED_FOR);
    }

    fn closed(&self, id: u32, port: Port) -> bool {
        lock(&self.closed)
            .get(&(id, port))
            .is_some_and(|until| Instant::now() < *until)
    }
}

fn manager(shared: &Shared) -> MutexGuard<'_, crate::manager::Manager<super::machine::Machine>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One connection, until it goes.
fn connection(
    shared: &Shared,
    links: &Links,
    id: u32,
    port: Port,
    stream: &UnixStream,
    sequence: u64,
) {
    let (to, from) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let Ok(kept) = stream.try_clone() else {
        return;
    };
    let number = {
        let mut n = lock(&links.numbers);
        *n += 1;
        *n
    };
    lock(&links.open).insert(
        (id, port),
        Open {
            to,
            stream: kept,
            number,
        },
    );
    let writer = thread::spawn(move || {
        let mut out = writer_stream;
        let _ = out.set_write_timeout(Some(Duration::from_secs(5)));
        for frame in from {
            if out.write_all(&frame).is_err() {
                let _ = out.shutdown(Shutdown::Both);
                break;
            }
        }
    });
    manager(shared).channel_up(id, port);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut buf = vec![0u8; PIECE];
    let mut reader = stream;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => manager(shared).channel_bytes(id, port, &buf[..n]),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if !manager(shared).channel_wanted(id, sequence) {
                    break;
                }
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    {
        let mut open = lock(&links.open);
        if open.get(&(id, port)).is_some_and(|o| o.number == number) {
            open.remove(&(id, port));
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
    let _ = writer.join();
    manager(shared).channel_down(id, port);
}

/// Keep `id`'s connection to `port`'s socket at `path` while its VMM of
/// this activation runs.
fn keep(shared: &Shared, links: &Links, id: u32, port: Port, path: &PathBuf) {
    // The activation the VMM was started for: it is recorded once the
    // request that started it is done.
    let mut sequence = None;
    for _ in 0..100 {
        sequence = manager(shared).activation(id);
        if sequence.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let Some(sequence) = sequence else {
        return;
    };
    while manager(shared).channel_wanted(id, sequence) {
        if links.closed(id, port) {
            thread::sleep(RETRY);
            continue;
        }
        match UnixStream::connect(path) {
            Ok(stream) => connection(shared, links, id, port, &stream, sequence),
            Err(_) => thread::sleep(RETRY),
        }
    }
}

/// Keep both ports' connections of `id`, each on a thread.
pub fn watch(shared: &Shared, links: &Links, id: u32, sockets: [(Port, PathBuf); 2]) {
    for (port, path) in sockets {
        let (shared, links) = (shared.clone(), links.clone());
        thread::spawn(move || keep(&shared, &links, id, port, &path));
    }
}
