// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The sending side. Records wait in a queue, in order, until the log
//! service takes them: before it is up, while it restarts, and when its
//! socket is full. A send never blocks, so PID 1 can use it.

use std::collections::VecDeque;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

use crate::record::Record;

/// The most records kept waiting; past it the oldest are dropped.
const QUEUE_MAX: usize = 8192;

/// A queue of records and the socket they leave through.
#[derive(Debug)]
pub struct Sender {
    path: PathBuf,
    socket: Option<UnixDatagram>,
    queue: VecDeque<Record>,
    dropped: u64,
}

impl Sender {
    /// A sender to the log service's socket at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            socket: None,
            queue: VecDeque::new(),
            dropped: 0,
        }
    }

    /// Queue a record and send what can be sent.
    pub fn send(&mut self, record: Record) {
        if self.queue.len() >= QUEUE_MAX {
            self.queue.pop_front();
            self.dropped += 1;
        }
        self.queue.push_back(record);
        self.flush();
    }

    /// Send what can be sent, oldest first. True when nothing waits.
    pub fn flush(&mut self) -> bool {
        if self.queue.is_empty() {
            return true;
        }
        if self.socket.is_none() {
            self.socket = UnixDatagram::unbound()
                .and_then(|s| s.set_nonblocking(true).map(|()| s))
                .ok();
        }
        let Some(socket) = &self.socket else {
            return false;
        };
        while let Some(record) = self.queue.front() {
            // No service yet, or its queue is full: try again later.
            if socket.send_to(&ipc::encode(record), &self.path).is_err() {
                return false;
            }
            self.queue.pop_front();
        }
        true
    }

    /// How many records wait.
    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// How many records were dropped because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::Severity;

    #[test]
    fn records_wait_for_the_service_and_arrive_in_order() {
        let path = std::env::temp_dir().join(format!("logd-client-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut sender = Sender::new(&path);
        for n in 0..3 {
            sender.send(Record::new("t", Severity::Info, &format!("early {n}")));
        }
        assert_eq!(sender.waiting(), 3);
        let service = UnixDatagram::bind(&path).unwrap();
        sender.send(Record::new("t", Severity::Info, "late"));
        assert_eq!(sender.waiting(), 0);
        let mut buf = vec![0u8; 65536];
        let mut got = Vec::new();
        for _ in 0..4 {
            let n = service.recv(&mut buf).unwrap();
            got.push(ipc::decode::<Record>(&buf[..n]).unwrap().message);
        }
        assert_eq!(got, ["early 0", "early 1", "early 2", "late"]);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_full_queue_drops_the_oldest() {
        let mut sender = Sender::new("/nonexistent/logd.sock");
        for n in 0..QUEUE_MAX + 2 {
            sender.send(Record::new("t", Severity::Info, &n.to_string()));
        }
        assert_eq!((sender.waiting(), sender.dropped()), (QUEUE_MAX, 2));
        assert_eq!(sender.queue.front().unwrap().message, "2");
    }
}
