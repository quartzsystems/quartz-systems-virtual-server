// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The far side of the partition's serial port. What the guest
//! transmits goes to a log file, rotated by size, and to whoever is
//! connected to the console socket; what they send is the guest's
//! input. A later phase attaches a console to the socket.
//!
//! - [`log`]: `<data>/log/partitions/<name>/serial.log` and its
//!   generations.
//! - [`console`]: the Unix socket.
//!
//! The guest's processors never wait on the disk or on a client: they
//! put bytes in a bounded queue and one thread takes them out. When
//! the queue is full the guest waits, as it would on a slow line.

pub mod console;
pub mod log;

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::Log;

/// Bytes the queue holds before the guest waits.
const QUEUE_BYTES: usize = 64 * 1024;
/// The most bytes written at once.
const BATCH_BYTES: usize = 4096;
/// How long a client may take to accept output before it is dropped.
const CLIENT_PATIENCE: Duration = Duration::from_millis(200);

/// What is put in the queue.
pub enum Message {
    /// A byte the guest transmitted.
    Byte(u8),
    /// Everything before this is to be written out; then say so.
    Flush(Sender<()>),
}

/// The clients connected to the console socket.
pub type Clients = Arc<Mutex<Vec<UnixStream>>>;

/// The queue's writing end: the serial port's output.
#[derive(Clone)]
pub struct Tap(SyncSender<Message>);

impl Write for Tap {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for byte in buf {
            self.0
                .send(Message::Byte(*byte))
                .map_err(|_| io::Error::other("the serial output has ended"))?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Tap {
    /// Wait, for no longer than `patience`, until everything the guest
    /// has transmitted is in the log.
    pub fn drain(&self, patience: Duration) {
        let (done, wait) = mpsc::channel();
        if self.0.send(Message::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(patience);
        }
    }
}

/// The queue: its writing end, and the reading end for [`spawn`].
pub fn queue() -> (Tap, Receiver<Message>) {
    let (tx, rx) = mpsc::sync_channel(QUEUE_BYTES);
    (Tap(tx), rx)
}

fn write_out(bytes: &[u8], log: &mut Log, clients: &Clients) {
    if bytes.is_empty() {
        return;
    }
    log.write(bytes);
    if let Ok(mut clients) = clients.lock() {
        // A client that does not take the output is dropped.
        clients.retain_mut(|client| client.write_all(bytes).is_ok());
    }
}

/// Start the thread that takes the guest's output from the queue.
pub fn spawn(
    queue: Receiver<Message>,
    mut log: Log,
    clients: Clients,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("serial-out".into())
        .spawn(move || {
            let mut batch = Vec::with_capacity(BATCH_BYTES);
            while let Ok(first) = queue.recv() {
                let mut next = Some(first);
                while let Some(message) = next.take() {
                    match message {
                        Message::Byte(byte) => batch.push(byte),
                        Message::Flush(done) => {
                            write_out(&batch, &mut log, &clients);
                            batch.clear();
                            log.sync();
                            let _ = done.send(());
                        }
                    }
                    if batch.len() < BATCH_BYTES {
                        next = queue.try_recv().ok();
                    }
                }
                write_out(&batch, &mut log, &clients);
                batch.clear();
            }
        })
}

/// Make `client` one that cannot hold the output up.
pub fn patient(client: &UnixStream) {
    let _ = client.set_write_timeout(Some(CLIENT_PATIENCE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn output_reaches_the_log_and_the_clients_in_order() {
        let dir = std::env::temp_dir().join(format!("qsvmm-serial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let log = Log::open(&dir, 1 << 20).unwrap();
        let clients: Clients = Arc::default();
        let (mut ours, theirs) = UnixStream::pair().unwrap();
        clients.lock().unwrap().push(theirs);
        // A client that has gone away is dropped without harm.
        let (gone, kept) = UnixStream::pair().unwrap();
        drop(gone);
        clients.lock().unwrap().push(kept);

        let (mut tap, queue) = queue();
        spawn(queue, log, clients.clone()).unwrap();
        tap.write_all(b"REF C1001000 IPL started\n").unwrap();
        tap.write_all(b"second line\n").unwrap();
        tap.drain(Duration::from_secs(5));

        let text = std::fs::read_to_string(dir.join(log::FILE)).unwrap();
        assert_eq!(text, "REF C1001000 IPL started\nsecond line\n");
        let mut got = vec![0u8; text.len()];
        ours.read_exact(&mut got).unwrap();
        assert_eq!(got, text.as_bytes());
        assert_eq!(clients.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
